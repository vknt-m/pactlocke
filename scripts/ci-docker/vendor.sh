# Sourced by scripts/ci-docker.sh and scripts/rig.sh: makes the Docker
# volume of vendored crates match Cargo.lock. Crates are vendored here and
# copied in as a tar (again only when Cargo.lock changes), so containers
# build offline and never read crates through Docker's slow file sharing.
#   ensure_vendor IMAGE VOLUME
ensure_vendor() {
	local image="$1" volume="$2" vendored
	vendored="$(docker run --rm --network none -v "$volume:/vendor" "$image" cat /vendor/.cargo-lock 2>/dev/null || true)"
	if [ "$vendored" = "$(cat Cargo.lock)" ]; then
		echo "the volume matches Cargo.lock"
		return
	fi
	echo "vendoring here, then copying into the volume"
	rm -rf target/ci-docker/vendor
	mkdir -p target/ci-docker
	cargo vendor --locked --versioned-dirs --quiet target/ci-docker/vendor >/dev/null
	cp Cargo.lock target/ci-docker/vendor/.cargo-lock
	tar -C target/ci-docker/vendor -cf - . |
		docker run -i --rm --network none -v "$volume:/vendor" "$image" \
			bash -c 'find /vendor -mindepth 1 -delete && tar -C /vendor -xf -'
	echo "copied"
}
