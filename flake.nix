# Build the `sandman` binary. Use:
#
#   nix build         # produces ./result/bin/sandman
#   nix run           # same as running `sandman` with no args
#   nix run -- bench  # forwards args to sandman
#
# First build: `cargoHash` is a placeholder. Run `nix build` once; Nix
# prints the correct hash in the error; paste it back. From then on
# the build is offline and reproducible from ./Cargo.lock + cargoHash.
{
  description = "Sandman — an agent swarm that coordinates through a shared queue";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        cargo = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = cargo.package.name;
          version = cargo.package.version;

          src = pkgs.lib.cleanSource ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # First build prints the right value; paste it back here.
          cargoHash = "";

          # `reqwest` uses `rustls-tls-native-roots`, which on Linux reads
          # `$SSL_CERT_FILE`. Nix has no /etc/ssl/certs, so point it at
          # nixpkgs' cacert bundle at runtime.
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postInstall = ''
            wrapProgram $out/bin/sandman \
              --set SSL_CERT_FILE "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
          '';

          meta = {
            description = cargo.package.description;
            mainProgram = "sandman";
            platforms = pkgs.lib.platforms.unix;
          };
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/sandman";
        };
      });
}
