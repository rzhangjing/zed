{
  # Merged: the Octos workspace (default outputs) plus the gpui-component /
  # Zed outputs this flake used to provide (now under `packages.gpui-component-story`
  # and `devShells.gpui` instead of the deprecated `defaultPackage` / the
  # default dev shell).
  description = "Octos - Agentic OS (merged with the Zed / gpui-component tree)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    # Kept from the gpui-component flake: the merged outputs below no longer
    # need it, but existing `inputs.flake-utils` references keep resolving.
    flake-utils.url = "github:numtide/flake-utils";
    # Octos' inputs.
    nix-darwin = {
      url = "https://flakehub.com/f/nix-darwin/nix-darwin/*";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, ... }@inputs:
    let
      # Union of the systems Octos supported (x86_64-linux, aarch64-linux,
      # aarch64-darwin) and the ones the previous flake covered through
      # flake-utils' eachDefaultSystem (adds x86_64-darwin).
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      forEachSupportedSystem =
        f:
        inputs.nixpkgs.lib.genAttrs supportedSystems (
          system:
          f {
            inherit system;
            # Plain nixpkgs: what ./nix/*.nix (Octos) was written against.
            pkgs = import inputs.nixpkgs { inherit system; };
            # rust-overlay, for the gpui-component shell and package below.
            pkgsRust = import inputs.nixpkgs {
              inherit system;
              overlays = [ (import inputs.rust-overlay) ];
            };
          }
        );

      treefmtEval = forEachSupportedSystem (
        { pkgs, ... }: inputs.treefmt-nix.lib.evalModule pkgs ./nix/treefmt.nix
      );

      mkModuleWithPackages =
        modulePath:
        { pkgs, lib, ... }:
        let
          inherit (pkgs.stdenv.hostPlatform) system;
        in
        {
          imports = [
            (lib.modules.importApply ./nix/modules/options.nix {
              inherit (self.packages.${system}) octos;
            })
            modulePath
          ];
        };

      perSystem = forEachSupportedSystem (
        { pkgs, pkgsRust, system }:
        let
          octos = pkgs.callPackage ./nix/packages/default.nix { };

          # gpui-component / Zed: what this repository's flake used to build.
          gpuiUiFont = pkgsRust.ibm-plex.override {
            families = [ "sans" ];
          };
          gpuiFontsConf = pkgsRust.makeFontsConf {
            fontDirectories = [ gpuiUiFont ];
          };
          gpuiBuildDependencies = with pkgsRust; [
            pkg-config # For dynamically linked libraries
            makeWrapper # To provide LD_LIBRARY_PATH to the final binary
            (rust-bin.beta.latest.default.override {
              extensions = [ "rust-src" ];
            })
          ];
          gpuiDynamicLibraries = with pkgsRust; [
            wayland
            vulkan-headers
            vulkan-loader
            libxcb
            libxkbcommon
            atk
            fontconfig
            gio-sharp
            glib
            gtk3
          ];
        in
        {
          packages = {
            inherit octos;
            default = octos;
            octos-minimal = octos;
            octos-full = octos.override {
              enableAllFeatures = true;
              enableAppSkills = true;
            };

            # Formerly this flake's `defaultPackage`.
            gpui-component-story = pkgsRust.rustPlatform.buildRustPackage (finalAttrs: {
              pname = "gpui-component-story";
              version = "0.5.1";
              src = ./.;
              cargoLock = {
                lockFile = ./Cargo.lock;
                allowBuiltinFetchGit = true;
              };
              nativeBuildInputs = gpuiBuildDependencies;
              buildInputs = gpuiDynamicLibraries;
              postFixup = ''
                wrapProgram $out/bin/gpui-component-story \
                --suffix LD_LIBRARY_PATH : ${pkgsRust.lib.makeLibraryPath gpuiDynamicLibraries} \
                --set FONTCONFIG_FILE ${gpuiFontsConf}
              '';
            });
          };

          devShells = {
            # Octos' development environment is the default for the merged
            # repository.
            default = pkgs.callPackage ./nix/shell.nix { };

            # Formerly this flake's `devShells.default`.
            gpui = pkgsRust.mkShell {
              buildInputs = gpuiBuildDependencies ++ gpuiDynamicLibraries;
              env = {
                RUST_BACKTRACE = "1";
                FONTCONFIG_FILE = gpuiFontsConf;
                LD_LIBRARY_PATH = pkgsRust.lib.makeLibraryPath gpuiDynamicLibraries;
              };
            };
          };

          checks = {
            formatting = treefmtEval.${system}.config.build.check self;

            # Darwin module evaluation (cross-platform).
            darwin-module = pkgs.callPackage ./nix/tests/darwin.nix {
              inherit (inputs) nix-darwin;
              octosModule = self.darwinModules.default;
            };
          } // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            # NixOS VM test (Linux only, full E2E).
            nixos-module-vm = pkgs.callPackage ./nix/tests/nixos.nix {
              octosModule = self.nixosModules.default;
            };
          };
        }
      );
    in
    {
      formatter = forEachSupportedSystem (
        { system, ... }: treefmtEval.${system}.config.build.wrapper
      );

      nixosModules.default = mkModuleWithPackages ./nix/modules/nixos.nix;
      darwinModules.default = mkModuleWithPackages ./nix/modules/darwin.nix;

      packages = builtins.mapAttrs (_: v: v.packages) perSystem;
      devShells = builtins.mapAttrs (_: v: v.devShells) perSystem;
      checks = builtins.mapAttrs (_: v: v.checks) perSystem;
    };
}
