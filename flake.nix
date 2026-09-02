{
	description = "And AI agent swarm that communicates through tasks";

	inputs = {
		nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
		flake-utils.url = "github:numtide/flake-utils";
	};

	outputs = { self, nixpkgs, flake-utils }:
		flake-utils.lib.eachDefaultSystem (system:
			rec {
				packages.sandman = rustPlatform.buildRustPackage {
					pname = "sandman";
					version = "4.1.0";
					src = ./.;
					cargoHash = "";
				};
				defaultPackage = packages.sandman;
			});
}

