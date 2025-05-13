{
  description = "tx-pigeon Rust project flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;

        tx-pigeon = craneLib.buildPackage {
          inherit src;
          pname = "tx-pigeon";
        };
      in
      {
        packages.tx-pigeon = tx-pigeon;
        defaultPackage = tx-pigeon;

        devShells.default = craneLib.devShell {
          packages = [
            pkgs.rust-analyzer
            pkgs.clippy
            pkgs.rustfmt
          ];
        };
      }
    );
}