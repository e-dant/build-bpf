# build-bpf

Tools for building ELFs and Skeletons for typical (e)BPF programs.

Usage for a project which wants to build BPF targets in `src/bpf/*.bpf.c`.

```rust
// build.rs
fn main() {
    build_bpf::guess_targets().for_each(|target| target.must_build());
}
```

To symlink the generated (Rust) Skeleton files, you can do something like this:

```rust
// build.rs
fn main() {
    let ln_to = |target: &build_bpf::BuildBpf| {
        format!(
            "{}/skel_{}.rs",
            build_bpf::cargo_crate_manifest_dir(),
            target.bpf_prog_src_file
        )
    };
    build_bpf::guess_targets().for_each(|target| {
        target.must_build().must_sym_link_skel_to(&ln_to(&target));
    });
}
```

Having the generated Skeleton files around can, instead of digging through `target/...`, can be useful during development.
