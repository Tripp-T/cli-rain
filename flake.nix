{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }: 
    flake-utils.lib.eachDefaultSystem
      (system: let
        overlays = [
          (import rust-overlay)
          (final: prev: {
            rust-toolchain = prev.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" "rustfmt" ];
            };
          })
        ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        cargoToml = builtins.fromTOML (builtins.readFile (self + /Cargo.toml));
        inherit (cargoToml.package) name version;

        buildInputs = with pkgs; [
          rust-toolchain
          openssl
          pkg-config
        ];

        craneBuild = rec {
          craneLib = (crane.mkLib pkgs).overrideToolchain pkgs.rust-toolchain;
          args = {
            inherit buildInputs version;
            src = pkgs.lib.cleanSourceWith {
              src = self; # The original, unfiltered source
              filter = path: type:
                # Default filter from crane (allow .rs files)
                (craneLib.filterCargoSources path type);
              };
            pname = name;
          };
          cargoArtifacts = craneLib.buildDepsOnly args;
          buildArgs = args // {
            inherit cargoArtifacts;
            cargoTestCommand = "cargo test --release -vvv";
            nativeBuildInputs = with pkgs; [];
            installPhaseCommand = ''
              mkdir -p $out/bin
              cp target/release/${name} $out/bin/
            '';
          };
          package = craneLib.buildPackage (buildArgs);
          check = craneLib.cargoClippy (args // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets --all-features -- --deny warnings";
          });
          doc = craneLib.cargoDoc (args // {
            inherit cargoArtifacts;
          });
        };
      in {
        devShells.default = pkgs.mkShell {
          inherit buildInputs;
          packages = with pkgs; [
            cargo-watch
          ];
          RUST_BACKTRACE = 1;
          RUST_LOG = "debug";
        };
        packages.default = craneBuild.package;
        checks.default = craneBuild.check;
      });
}
