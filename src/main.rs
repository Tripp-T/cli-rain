use {
    anyhow::{bail, Context as _, Result},
    clap::Parser,
    colored::Colorize,
    crossterm::{
        event::{poll, read, Event, KeyCode},
        execute, terminal,
    },
    itertools::Itertools,
    rand::prelude::*,
    rayon::prelude::*,
    std::{
        io::{stdout, Write},
        ops::RangeInclusive,
        process::exit,
        time::Duration,
    },
    tracing::debug,
    tracing_subscriber::EnvFilter,
};

#[derive(Parser)]
struct Opts {
    #[clap(long)]
    /// Disables color (used to show depth, lighter is closer)
    no_color: bool,
    #[clap(short = 'r', long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=100))]
    /// How likely a new raindrop is to spawn in each top column every update (1-100)
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
    ctrlc::set_handler(handle_exit).context("failed to set ctrl-c handler")?;

    let window_size = terminal::size().context("failed to get terminal window size")?;
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
            stdout.flush().context("failed to flush stdout")?;
            rain_map.update();
            rain_map.hydrate(&opts);
        }
    }
}

fn handle_exit() {
    execute!(std::io::stdout(), terminal::LeaveAlternateScreen)
        .expect("failed to exit alternate screen");
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
                self.entities.push((
                    Pos::new(x as i32, 0, rand.random_range(-16384..16384)),
                    RainEntity::new(&mut rand),
                ));
            }
        }
    }
    pub fn contains(&self, pos: &Pos) -> bool {
        pos.x >= 0 && pos.x < self.width as i32 && pos.y >= 0 && pos.y < self.height as i32
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
                    // normalize z (i16) to a u8
                    let normalized_z = (((z as i32 - i16::MIN as i32) * 255)
                        / (i16::MAX as i32 - i16::MIN as i32))
                        as u8;

                    format!("{c}")
                        .truecolor(0, normalized_z.checked_div(2).unwrap_or(0), normalized_z)
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
impl RainEntity {
    const AVAILABLE_CHARS: &[char] = &['\\', '/', '|', '~', '(', ')', '[', ']', '*', '#', '@'];
    pub fn new(rand: &mut ThreadRng) -> Self {
        Self {
            c: Self::AVAILABLE_CHARS[rand.random_range(0..Self::AVAILABLE_CHARS.len())],
            velocity: Velocity::new(rand),
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
    const Z_RANGE: RangeInclusive<i16> = -5248..=5248;
    pub fn new(rand: &mut ThreadRng) -> Self {
        Self {
            x: rand.random_range(Self::X_RANGE),
            y: rand.random_range(Self::Y_RANGE),
            z: rand.random_range(Self::Z_RANGE),
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
        self.y -= vel.y;
        self.z = self.z.saturating_add(vel.z);
    }
}
