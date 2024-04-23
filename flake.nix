{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      self,
      flake-parts,
      rust-overlay,
      nixpkgs,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        { pkgs, system, ... }:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            openssl
            rustPlatform.bindgenHook
          ];

          buildInputs = with pkgs; [
            vulkan-loader
            libGL
            udev
            alsa-lib
            libclang
            v4l-utils

            #wayland
            libxkbcommon
            wayland

            #x11
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
            xorg.libxcb
            xorg.libX11
          ];

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;

          devPackages = with pkgs; [
            (rust-bin.stable.latest.default.override {
              extensions = [
                "cargo"
                "clippy"
                "rust-src"
                "rust-analyzer"
                "rustc"
                "rustfmt"
              ];
            })
            cargo-flamegraph
          ];
        in
        {
          devShells = {
            default = pkgs.mkShell {
              inherit LD_LIBRARY_PATH buildInputs nativeBuildInputs;
              packages = devPackages;
            };
          };
        };
    };
}
