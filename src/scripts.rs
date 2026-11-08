use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub struct Scripts {
    pub directory: PathBuf,
}
impl Scripts {
    pub fn new(directory: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        Ok(Self { directory })
    }
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|s| s == "lua")
            {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_owned());
                }
            }
        }
        names.sort();
        Ok(names)
    }
    pub fn path(&self, name: &str) -> io::Result<PathBuf> {
        let name = name.trim();
        if name.is_empty()
            || name.len() > 120
            || name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
            || name.ends_with('.')
            || Path::new(name).file_stem().is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "请输入合法的脚本文件名（不能包含路径）",
            ));
        }
        let name = if name.ends_with(".lua") {
            name.to_owned()
        } else {
            format!("{name}.lua")
        };
        let stem = name.split('.').next().unwrap().to_uppercase();
        if [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "文件名是 Windows 保留名称",
            ));
        }
        Ok(self.directory.join(name))
    }
    pub fn save(&self, name: &str, source: &str) -> io::Result<()> {
        fs::write(self.path(name)?, source)
    }
    pub fn create(&self, name: &str, source: &str) -> io::Result<()> {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path(name)?)?;
        file.write_all(source.as_bytes())
    }
    pub fn rename(&self, old: &str, new: &str) -> io::Result<()> {
        let target = self.path(new)?;
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "同名脚本已存在",
            ));
        }
        fs::rename(self.path(old)?, target)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_paths_and_reserved_names() {
        let scripts = Scripts {
            directory: PathBuf::from("scripts"),
        };
        for name in ["../oops", "C:\\oops", "", "CON", "NUL.lua", "a/b", ".."] {
            assert!(scripts.path(name).is_err(), "{name}");
        }
        assert_eq!(
            scripts.path("示例").unwrap(),
            PathBuf::from("scripts/示例.lua")
        );
    }
}
