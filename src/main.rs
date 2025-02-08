#![allow(unused_imports)]
use {
    anyhow::{bail, Context as _, Result},
    clap::Parser,
    crossterm::{
        cursor,
        event::{poll, read, Event, KeyCode, KeyEvent, KeyEventKind},
        execute, queue,
        style::Stylize,
        terminal, QueueableCommand,
    },
    itertools::Itertools,
    once_cell::sync::Lazy,
    rand::{distr, prelude::*, rng},
    rayon::prelude::*,
    std::{
        fmt::Display,
        io::{stdout, Write},
        ops::{Neg, RangeInclusive},
        process::exit,
        time::Duration,
    },
    tracing::debug,
    tracing_subscriber::EnvFilter,
};

#[derive(Parser)]
struct Opts {
    #[clap(long)]
    /// Disables color (used to show depth)
    no_color: bool,
    #[clap(short = 'r', long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=100))]
    /// How likely a new raindrop is to spawn in a column (1-100)
    spawn_rate: u8,
    #[clap(short, long, default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..=2000))]
    /// How frequently to update the screen (in milliseconds)
    update_rate: u64,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let opts = Opts::parse();

    let mut stdout = stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;
    // execute!(stdout, terminal::Clear(terminal::ClearType::All))?;
    // execute!(stdout, cursor::MoveTo(0, 0))?;
    let window_size = terminal::size().context("failed to get terminal window size")?;

    ctrlc::set_handler(handle_exit).context("failed to set ctrl-c handler")?;

    debug!("window size: {}x{}", window_size.0, window_size.1);

    let mut rain_map = RainMap::new(window_size.0 as usize, window_size.1 as usize)?;
    rain_map.hydrate(&opts);

    loop {
        if poll(Duration::from_millis(opts.update_rate))? {
            match read()? {
                Event::Resize(width, height) => rain_map.resize(width as usize, height as usize)?,
                Event::Key(key) => {
                    if key == KeyCode::Char('q').into() || key == KeyCode::Esc.into() {
                        handle_exit();
                    }
                }
                e => debug!("unhandled event: {e:?}"),
            }
        } else {
            // no terminal events, loop as usual
            write!(stdout, "{}", rain_map.render(&opts))?;
            rain_map.update();
            rain_map.hydrate(&opts);
            stdout.flush().context("failed to flush stdout")?;
        }
    }
}

fn handle_exit() {
    let mut stdout = std::io::stdout();
    queue!(stdout, terminal::LeaveAlternateScreen).expect("failed to exit alternate screen");
    stdout
        .flush()
        .context("failed to flush stdout")
        .expect("failed to flush stdout");
    exit(0);
}

struct RainMap {
    entities: Vec<(Pos, RainEntity)>,
    height: usize,
    width: usize,
}
impl RainMap {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("width and height must be greater than 0");
        }
        Ok(Self {
            entities: Vec::new(),
            width,
            height,
        })
    }
    /// Adds new rain entities to the top of the map
    pub fn hydrate(&mut self, opts: &Opts) {
        let mut rand = rand::rng();
        for x in 0..self.width {
            let should_add = rand.random_bool(opts.spawn_rate as f64 / 100.0);
            if should_add {
                self.entities
                    .push((Pos::new(x as i32, 0, 0), RainEntity::new()));
            }
        }
    }
    pub fn contains(&self, pos: &Pos) -> bool {
        pos.x > 0 && pos.x < self.width as i32 && pos.y > 0 && pos.y < self.height as i32
    }
    pub fn resize(&mut self, width: usize, height: usize) -> Result<()> {
        debug!(
            "resizing from {}x{} to {}x{}",
            self.width, self.height, width, height
        );
        if width == 0 || height == 0 {
            bail!("width and height must be greater than 0");
        }
        self.width = width;
        self.height = height;

        let entities = self.entities.drain(..).collect_vec();
        self.entities = entities
            .into_par_iter()
            .filter(|(p, _)| self.contains(p))
            .collect();
        Ok(())
    }
    /// Runs the rain simulation
    pub fn update(&mut self) {
        let entities = self.entities.drain(..).collect_vec();
        self.entities = entities
            .into_par_iter()
            .filter_map(|(mut p, e)| {
                p.shift(&e.velocity);
                if self.contains(&p) {
                    Some((p, e))
                } else {
                    None
                }
            })
            .collect();
    }
    pub fn render(&self, opts: &Opts) -> String {
        let mut data = Vec::<Vec<Option<(i16, char)>>>::new();
        for _ in 0..self.height {
            let mut row = Vec::new();
            for _ in 0..self.width {
                row.push(None);
            }
            data.push(row);
        }
        for (p, e) in self.entities.iter().filter(|(p, _)| self.contains(p)) {
            let data_entry = &mut data[p.y as usize][p.x as usize];
            let Some((z, c)) = data_entry else {
                *data_entry = Some((p.z, e.c));
                continue;
            };
            if *z > p.z {
                // current entry is higher than canidate
                continue;
            }
            *z = p.z;
            *c = e.c;
        }
        let mut output = String::new();
        for row in data {
            for col in row {
                let Some((z, c)) = col else {
                    output.push(' ');
                    continue;
                };
                let s = if !opts.no_color {
                    match (z, format!("{c}")) {
                        (..0, s) => s.dark_blue(),
                        (0, s) => s.blue(),
                        (_, s) => s.cyan(),
                    }
                    .to_string()
                } else {
                    format!("{c}")
                };
                output.push_str(&s);
            }
            output.push('\n');
        }
        output
    }
}

#[derive(Debug, Clone, Copy)]
struct RainEntity {
    c: char,
    velocity: Velocity,
}
impl Display for RainEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.c)
    }
}
impl RainEntity {
    const AVAILABLE_CHARS: &[char] = &['\\', '/', '|', '~', '(', ')', '[', ']', '*', '#', '@'];
    pub fn new() -> Self {
        let mut rand = rand::rng();
        Self {
            c: Self::AVAILABLE_CHARS[rand.random_range(0..Self::AVAILABLE_CHARS.len())],
            velocity: Velocity::new(&mut rand),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Velocity {
    x: i32,
    y: i32,
    z: i16,
}
impl Velocity {
    const X_RANGE: RangeInclusive<i32> = -3..=3;
    const Y_RANGE: RangeInclusive<i32> = -3..=-1;
    const Z_RANGE: RangeInclusive<i16> = -3..=3;
    pub fn new(rand: &mut ThreadRng) -> Self {
        Self {
            x: rand.random_range(Self::X_RANGE).neg(),
            y: rand.random_range(Self::Y_RANGE).neg(),
            z: rand.random_range(Self::Z_RANGE).neg(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Pos {
    x: i32,
    y: i32,
    z: i16,
}
impl Pos {
    pub fn new(x: i32, y: i32, z: i16) -> Self {
        Self { x, y, z }
    }
    pub fn shift(&mut self, vel: &Velocity) {
        self.x += vel.x;
        self.y += vel.y;
        self.z = self.z.saturating_add(vel.z);
    }
}
