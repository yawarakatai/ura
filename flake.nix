{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      self,
      flake-parts,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      flake.homeManagerModules = rec {
        default = import ./nix/home-manager-module.nix { inherit self; };
        ura = default;
      };

      perSystem =
        {
          pkgs,
          inputs',
          ...
        }:
        let
          fenixPkgs = inputs'.fenix.packages;

          rustToolchain = fenixPkgs.stable.withComponents [
            "cargo"
            "clippy"
            "rust-src"
            "rustc"
            "rustfmt"
          ];

          rustPlatform = pkgs.makeRustPlatform {
            cargo = fenixPkgs.stable.cargo;
            rustc = fenixPkgs.stable.rustc;
          };

          uraPackage = rustPlatform.buildRustPackage {
            pname = "ura";
            version = "0.6.0";

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [
              pkgs.pkg-config
            ];

            buildInputs = [
              pkgs.sqlite
            ];
          };
        in
        {
          packages = {
            default = uraPackage;
            ura = uraPackage;
          };

          devShells.default = pkgs.mkShell {
            packages = [
              rustToolchain
              fenixPkgs.stable.rust-analyzer

              pkgs.mpv
              pkgs.sqlite
              pkgs.yt-dlp
            ];
          };
        };
    };
}
