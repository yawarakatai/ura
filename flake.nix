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
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        { pkgs, inputs', ... }:
        {
          devShells.default = pkgs.mkShell {
            packages = [
              # Rust (Fenix (stable))
              inputs'.fenix.packages.stable.toolchain
              inputs'.fenix.packages.stable.rust-analyzer
            ];
          };
        };
    };
}
