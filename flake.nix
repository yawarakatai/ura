{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
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
        { pkgs, ... }:
        let
          uraPackage = pkgs.rustPlatform.buildRustPackage {
            pname = "ura";
            version = "0.7.0";

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            # nixpkgs' cargoUseSystemLibs links the system SQLite even with
            # rusqlite's `bundled` feature.
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
              pkgs.cargo
              pkgs.clippy
              pkgs.rustc
              pkgs.rustfmt
              pkgs.rust-analyzer

              pkgs.mpv
              pkgs.sqlite
              pkgs.yt-dlp
            ];

            RUST_SRC_PATH = pkgs.rustPlatform.rustLibSrc;
          };
        };
    };
}
