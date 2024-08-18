use libbpf_cargo::SkeletonBuilder;

macro_rules! known_env {
    ($name:literal) => {
        std::env::var($name).expect(concat!($name, " must be set in build script"))
    };
}

enum DiffError {
    File1Read,
    File2Read,
    Io,
}

fn try_file_content_differs(file1: &str, file2: &str) -> Result<bool, DiffError> {
    use std::fs;
    use std::io::Read;
    let mut buf1 = Vec::new();
    let mut buf2 = Vec::new();
    let mut file1 = fs::File::open(file1).map_err(|_| DiffError::File1Read)?;
    let mut file2 = fs::File::open(file2).map_err(|_| DiffError::File2Read)?;
    if file1.read_to_end(&mut buf1).is_err() || file2.read_to_end(&mut buf2).is_err() {
        Err(DiffError::Io)
    } else {
        Ok(buf1 != buf2)
    }
}

fn sym_link_when_files_differ(from: &str, to: &str) -> Result<(), std::io::Error> {
    let differs = try_file_content_differs(from, to);
    match differs {
        Err(DiffError::File1Read) => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Link source file not found: {from}"),
        )),
        Ok(false) => Ok(()),
        _ => {
            std::fs::remove_file(to).ok();
            std::os::unix::fs::symlink(from, to)?;
            Ok(())
        }
    }
}

struct TmpDir {
    path: std::path::PathBuf,
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        if std::fs::metadata(&self.path).is_ok() {
            std::fs::remove_dir_all(&self.path).expect("Failed to remove temp dir");
        }
    }
}

impl TmpDir {
    fn new() -> Self {
        let cmd = std::process::Command::new("mktemp")
            .arg("-d")
            .output()
            .expect("Failed to run mktemp");
        if !cmd.status.success() {
            panic!("Failed to run mktemp");
        }
        let path = std::str::from_utf8(&cmd.stdout)
            .expect("Failed to parse mktemp output")
            .trim()
            .to_string();
        let path = std::path::PathBuf::from(path);
        if !std::fs::metadata(&path).is_ok() {
            panic!("Failed to create temp dir: {path:?}");
        }
        Self { path }
    }
}

// If the kernel headers don't exist, make them.
fn gen_vmlinux(dst_dir: &str) -> Result<(), std::io::Error> {
    let tmp = TmpDir::new();
    let tmp = tmp.path.clone();
    let tmp = tmp.to_str().unwrap();
    if std::fs::metadata(dst_dir).is_ok() {
        return Ok(());
    }
    if !std::fs::metadata("/sys/kernel/btf/vmlinux").is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error (BTF not enabled on this host)",
        ));
    }
    if !std::process::Command::new("which")
        .arg("bpftool")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error (bpftool not found)",
        ));
    }
    let gitcloned = std::process::Command::new("git")
        .arg("clone")
        .arg("https://github.com/libbpf/libbpf-bootstrap")
        .arg(format!("{tmp}/libbpf-bootstrap"))
        .output()?;
    if !gitcloned.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error (failed to clone) libbpf-bootstrap",
        ));
    }
    let vmlinux_src_dir = format!("{tmp}/libbpf-bootstrap/vmlinux");
    if !std::fs::metadata(&vmlinux_src_dir).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error (vmlinux source not found)",
        ));
    }
    let vmlinux_src_arch_dirs = std::process::Command::new("find")
        .arg(".")
        .arg("-mindepth")
        .arg("1")
        .arg("-maxdepth")
        .arg("1")
        .arg("-type")
        .arg("d")
        .current_dir(&vmlinux_src_dir)
        .output()?;
    let vmlinux_src_arch_dirs = std::str::from_utf8(&vmlinux_src_arch_dirs.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty());
    for arch in vmlinux_src_arch_dirs {
        let src = format!("{vmlinux_src_dir}/{arch}/vmlinux.h");
        let dst_dir = format!("{dst_dir}/{arch}");
        let src = std::path::Path::new(&src).canonicalize().unwrap();
        if std::fs::metadata(&dst_dir).is_err() {
            std::fs::create_dir_all(&dst_dir)?;
        }
        std::fs::rename(src, &format!("{dst_dir}/vmlinux.h"))?;
    }
    let vmlinux_host_dst_dir = format!("{dst_dir}/host");
    std::fs::create_dir_all(std::path::Path::new(&vmlinux_host_dst_dir))?;
    let vmlinux_h = format!("{vmlinux_host_dst_dir}/vmlinux.h");
    let bpftool = std::process::Command::new("bpftool")
        .arg("btf")
        .arg("dump")
        .arg("file")
        .arg("/sys/kernel/btf/vmlinux")
        .arg("format")
        .arg("c")
        .output()?;
    if !bpftool.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Error (bpftool failed to dump BTF)",
        ));
    }
    std::fs::write(&vmlinux_h, bpftool.stdout)?;
    Ok(())
}

// To build the elf manually:
// clang -g -target bpf -D__TARGET_ARCH_x86 -c src/bpf/<prog>.bpf.c
// Which can be nice to actually see the compiler errors.
fn gen_skel(
    prog_src_file: &str,
    vmlinux_hdr_dir: &str,
    skel_out_file: &str,
) -> Result<(), std::io::Error> {
    let cargo_arch = known_env!("CARGO_CFG_TARGET_ARCH");
    let kernel_arch = cargo_arch_to_kernel_arch(&cargo_arch);
    SkeletonBuilder::new()
        .source(prog_src_file)
        .debug(true)
        .clang_args(["-I", &format!("{vmlinux_hdr_dir}/{kernel_arch}")])
        .build_and_generate(std::path::Path::new(&skel_out_file))
        .map_err(|e| {
            println!("Failed to build BPF program: {e}");
            std::io::ErrorKind::Other
        })?;
    Ok(())
}

pub fn guess_bpf_prog_names(crate_manifest_dir: &str) -> impl std::iter::Iterator<Item = String> {
    std::fs::read_dir(&std::path::Path::new(&format!(
        "{crate_manifest_dir}/src/bpf"
    )))
    .unwrap()
    .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_string())
    .filter(|entry| entry.ends_with(".bpf.c"))
    .map(|entry| entry.split('.').next().unwrap().to_string())
}

pub fn guess_targets<'a>() -> impl std::iter::Iterator<Item = BuildBpf> + 'a {
    let crate_manifest_dir = known_env!("CARGO_MANIFEST_DIR");
    let cargo_out_dir = known_env!("OUT_DIR");
    guess_bpf_prog_names(&crate_manifest_dir).map(move |prog| {
        let src = format!("{crate_manifest_dir}/src/bpf/{prog}.bpf.c");
        let vmlinux_hdr_dir = format!("{crate_manifest_dir}/include/vmlinux");
        let skel_out_file = format!("{cargo_out_dir}/skel_{prog}.rs");
        let sym_link_skel_to = vec![format!("{crate_manifest_dir}/src/skel_{prog}.rs")];
        BuildBpf {
            bpf_prog_src_file: src,
            vmlinux_hdr_dir,
            skel_out_file,
            sym_link_skel_to,
        }
    })
}

pub fn cargo_arch_to_kernel_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "loongarch64" => "loongarch",
        "powerpc64" => "powerpc",
        "riscv64" => "riscv",
        "x86_64" => "x86",
        _ => "host",
    }
}

pub struct BuildBpf {
    bpf_prog_src_file: String,
    vmlinux_hdr_dir: String,
    skel_out_file: String,
    sym_link_skel_to: Vec<String>,
}

impl BuildBpf {
    pub fn try_build(&self) -> Result<&Self, std::io::Error> {
        println!("cargo:rerun-if-changed={}", self.bpf_prog_src_file);
        println!("cargo:rerun-if-changed={}", self.skel_out_file);
        gen_vmlinux(&self.vmlinux_hdr_dir)?;
        gen_skel(
            &self.bpf_prog_src_file,
            &self.vmlinux_hdr_dir,
            &self.skel_out_file,
        )?;
        for to in &self.sym_link_skel_to {
            println!("cargo:rerun-if-changed={to}");
            sym_link_when_files_differ(&self.skel_out_file, &to)?;
        }
        Ok(self)
    }

    pub fn must_build(&self) {
        self.try_build().unwrap();
    }
}
