{
	description = "An AI agent swarm that communicates through tasks";

	inputs = {
		nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
		flake-utils.url = "github:numtide/flake-utils";
	};

	outputs = { self, nixpkgs, flake-utils }:
		flake-utils.lib.eachDefaultSystem (system:
			let
				pkgs = import nixpkgs { inherit system; };
				lib = pkgs.lib;
			in {
				packages.default = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
					pname = "sandman";
					version = "4.1.0";

					src = ./.;
					# Replace with the hash `nix build` reports on first failure.
					cargoHash = lib.fakeHash;

					meta = {
						description = "An agent swarm that coordinates through a shared queue";
						maintainers = [ ];
					};
				});
			});
}