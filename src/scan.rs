use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub struct Node {
    pub name: OsString,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub bytes: u64,
    pub is_dir: bool,
    pub unreadable: bool,
}

pub struct Scan {
    pub nodes: Vec<Node>,
    pub errors: u64,
    pub files: u64,
    pub directories: u64,
}

fn disk_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

pub fn scan(root: &Path, progress: &AtomicU64) -> std::io::Result<Scan> {
    let root_meta = fs::symlink_metadata(root)?;
    let mut result = Scan {
        nodes: vec![Node {
            name: root.as_os_str().to_owned(),
            parent: None,
            children: Vec::new(),
            bytes: disk_bytes(&root_meta),
            is_dir: root_meta.is_dir(),
            unreadable: false,
        }],
        errors: 0,
        files: u64::from(!root_meta.is_dir()),
        directories: u64::from(root_meta.is_dir()),
    };
    let mut pending = Vec::new();
    if root_meta.is_dir() {
        pending.push((0, root.to_path_buf()));
    }
    #[cfg(unix)]
    let mut seen = HashSet::new();
    #[cfg(unix)]
    seen.insert((root_meta.dev(), root_meta.ino()));

    while let Some((parent, path)) = pending.pop() {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => {
                result.nodes[parent].unreadable = true;
                result.errors += 1;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.nodes[parent].unreadable = true;
                    result.errors += 1;
                    continue;
                }
            };
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => {
                    result.nodes[parent].unreadable = true;
                    result.errors += 1;
                    continue;
                }
            };
            let is_dir = meta.is_dir();
            let mut bytes = disk_bytes(&meta);
            #[cfg(unix)]
            if !seen.insert((meta.dev(), meta.ino())) {
                bytes = 0;
            }
            let index = result.nodes.len();
            result.nodes.push(Node {
                name: entry.file_name(),
                parent: Some(parent),
                children: Vec::new(),
                bytes,
                is_dir,
                unreadable: false,
            });
            result.nodes[parent].children.push(index);
            if is_dir {
                result.directories += 1;
                pending.push((index, path));
            } else {
                result.files += 1;
            }
            progress.fetch_add(1, Ordering::Relaxed);
        }
    }
    // Every child is appended after its parent; reverse order rolls sizes up once.
    for index in (1..result.nodes.len()).rev() {
        let parent = result.nodes[index]
            .parent
            .expect("non-root node has parent");
        result.nodes[parent].bytes = result.nodes[parent]
            .bytes
            .saturating_add(result.nodes[index].bytes);
    }
    for index in 0..result.nodes.len() {
        let mut children = std::mem::take(&mut result.nodes[index].children);
        children.sort_unstable_by(|&a, &b| {
            result.nodes[b]
                .bytes
                .cmp(&result.nodes[a].bytes)
                .then_with(|| result.nodes[a].name.cmp(&result.nodes[b].name))
        });
        result.nodes[index].children = children;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn ranks_nested_contents_and_does_not_follow_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "disk-explorer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("big")).unwrap();
        fs::write(root.join("big/data"), vec![42; 65_536]).unwrap();
        fs::write(root.join("small"), vec![42; 4096]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let progress = AtomicU64::new(0);
        let scan = scan(&root, &progress).unwrap();
        assert_eq!(scan.errors, 0);
        assert_eq!(scan.directories, 2);
        assert_eq!(scan.nodes[scan.nodes[0].children[0]].name, "big");
        assert!(scan.nodes[0].bytes >= scan.nodes[scan.nodes[0].children[0]].bytes);
        assert_eq!(
            progress.load(Ordering::Relaxed),
            scan.nodes.len() as u64 - 1
        );
        fs::remove_dir_all(root).unwrap();
    }
}
