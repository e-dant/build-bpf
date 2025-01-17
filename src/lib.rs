use libbpf_cargo::SkeletonBuilder;
#[cfg(feature = "vmlinux-archs")]
mod tmp_dir;

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

#[cfg(feature = "vmlinux-archs")]
fn gen_vmlinux_for_archs(dst_dir: &str) -> Result<(), std::io::Error> {
    let tmp = crate::tmp_dir::TmpDir::new();
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
    Ok(())
}

fn gen_vmlinux_for_host(dst_dir: &str) -> Result<(), std::io::Error> {
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

fn gen_vmlinux(dst_dir: &str) -> Result<(), std::io::Error> {
    #[cfg(feature = "vmlinux-archs")]
    {
        gen_vmlinux_for_archs(dst_dir)?;
    }
    gen_vmlinux_for_host(dst_dir)
}


fn vmlinux_include_dir(vmlinux_hdr_dir: &str, arch: &str) -> String {
    let archdir = format!("{vmlinux_hdr_dir}/{arch}");
    if std::fs::metadata(&archdir).is_ok() {
        archdir
    } else {
        format!("{vmlinux_hdr_dir}/host")
    }
}

fn gen_skel(
    prog_src_file: &str,
    vmlinux_hdr_dir: &str,
    skel_out_file: &str,
) -> Result<(), std::io::Error> {
    SkeletonBuilder::new()
        .source(prog_src_file)
        .debug(true)
        .clang_args(["-I", vmlinux_hdr_dir])
        .build_and_generate(std::path::Path::new(&skel_out_file))
        .map_err(|e| {
            println!(r#"
To build the elf manually:
$ clang -g -target bpf -D__TARGET_ARCH_x86 -c src/bpf/<prog>.bpf.c
Which can be nice to actually see the compiler errors.
Failed to build BPF program: {e}"#);
            std::io::ErrorKind::Other
        })?;
    Ok(())
}

fn cargo_crate_manifest_dir() -> String {
    known_env!("CARGO_MANIFEST_DIR")
}

fn cargo_out_dir() -> String {
    known_env!("OUT_DIR")
}

fn cargo_arch() -> String {
    known_env!("CARGO_CFG_TARGET_ARCH")
}

fn kernel_arch() -> String {
    cargo_arch_to_kernel_arch(&cargo_arch()).to_string()
}

fn guess_bpf_prog_names() -> impl std::iter::Iterator<Item = String> {
    let cratedir = cargo_crate_manifest_dir();
    std::fs::read_dir(&std::path::Path::new(&format!("{cratedir}/src/bpf")))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_string())
        .filter(|entry| entry.ends_with(".bpf.c"))
        .map(|entry| entry.split('.').next().unwrap().to_string())
}

fn cargo_arch_to_kernel_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "loongarch64" => "loongarch",
        "powerpc64" => "powerpc",
        "riscv64" => "riscv",
        "x86_64" => "x86",
        _ => "host",
    }
}

pub fn guess_targets<'a>() -> impl std::iter::Iterator<Item = BuildBpf> + 'a {
    guess_bpf_prog_names().map(move |prog| {
        let cratedir = cargo_crate_manifest_dir();
        let outdir = cargo_out_dir();
        let bpf_prog_src_file = format!("{cratedir}/src/bpf/{prog}.bpf.c");
        let vmlinux_base_dir = format!("{outdir}/include/vmlinux");
        let skel_dst_file = format!("{outdir}/skel_{prog}.rs");
        BuildBpf {
            bpf_prog_src_file,
            vmlinux_base_dir,
            skel_dst_file,
        }
    })
}

pub struct BuildBpf {
    bpf_prog_src_file: String,
    vmlinux_base_dir: String,
    skel_dst_file: String,
}

impl BuildBpf {
    pub fn bpf_prog_name(&self) -> String {
        let filename = std::path::Path::new(&self.bpf_prog_src_file)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        match filename.find('.') {
            Some(firstdot) => filename[..firstdot].to_string(),
            None => filename.to_string(),
        }
    }

    pub fn try_build(&self) -> Result<&Self, std::io::Error> {
        println!("cargo:rerun-if-changed={}", self.bpf_prog_src_file);
        gen_vmlinux(&self.vmlinux_base_dir)?;
        gen_skel(
            &self.bpf_prog_src_file,
            &vmlinux_include_dir(&self.vmlinux_base_dir, &kernel_arch()),
            &self.skel_dst_file,
        )?;
        Ok(self)
    }

    pub fn must_build(&self) -> &Self {
        self.try_build().unwrap()
    }

    pub fn try_sym_link_skel_to(&self, dst: &str) -> Result<&Self, std::io::Error> {
        println!("cargo:rerun-if-changed={dst}");
        sym_link_when_files_differ(&self.skel_dst_file, dst)?;
        Ok(self)
    }

    pub fn must_sym_link_skel_to(&self, dst: &str) -> &Self {
        self.try_sym_link_skel_to(dst).unwrap()
    }
}
