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

      flake = {
        homeManagerModules = {
          default = import ./nix/home-manager-module.nix {
            inherit self;
          };

          ura = self.homeManagerModules.default;
        };
      };

      perSystem =
        {
          pkgs,
          inputs',
          ...
        }:

        let
          rustPlatform = pkgs.makeRustPlatform {
            cargo = inputs'.fenix.packages.stable.cargo;
            rustc = inputs'.fenix.packages.stable.rustc;
          };
        in
        {
          packages.default = rustPlatform.buildRustPackage {
            pname = "ura";
            version = "0.1.0";

            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [
              pkgs.makeWrapper
              pkgs.pkg-config
            ];

            buildInputs = [
              pkgs.sqlite
            ];

            postInstall = ''
              wrapProgram "$out/bin/ura" \
                --prefix PATH : ${
                  pkgs.lib.makeBinPath [
                    pkgs.mpv
                    pkgs.yt-dlp
                  ]
                }
            '';
          };

          packages.ura = self.packages.${pkgs.stdenv.hostPlatform.system}.default;

          devShells.default = pkgs.mkShell {
            packages = [
              inputs'.fenix.packages.stable.toolchain
              inputs'.fenix.packages.stable.rust-analyzer
              pkgs.mpv
              pkgs.nodejs_22
              pkgs.sqlite
              pkgs.yt-dlp
            ];
          };
        };
    };
}
