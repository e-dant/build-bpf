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
