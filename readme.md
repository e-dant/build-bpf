# build-bpf

Tools for building ELFs and Skeletons for typical (e)BPF programs.

Usage for a project which wants to build BPF targets in `src/bpf/*.bpf.c` and symlink their ELF object files to `src/skel_*.rs`:

```rust
// build.rs
fn main() {
    build_bpf::guess_targets().for_each(|target| target.must_build());
}
```
