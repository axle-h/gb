//! The tile data the cartridge `INCBIN`s, generated from the committed `.png` sources by the build
//! script. A module per directory under `gfx/`; a compressed pic is here as its uncompressed
//! `.2bpp`, and a blockset, tilemap or run-length map as the committed file.

include!(concat!(env!("OUT_DIR"), "/gfx.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const ROOT: &str = "../vendor/pokered";

    /// Every asset matches what `make` left beside its `.png`, skipping loudly when those are absent.
    #[test]
    fn every_asset_matches_rgbgfx() {
        let converted: Vec<_> = ALL.iter().filter(|(path, _)| path.ends_with("bpp")).collect();
        let mut checked = 0;
        for (path, generated) in &converted {
            let Ok(expected) = std::fs::read(Path::new(ROOT).join(path)) else { continue };
            assert!(expected == *generated, "{path}: {} bytes generated, {} from rgbgfx, first difference at {:?}",
                generated.len(), expected.len(), generated.iter().zip(&expected).position(|(a, b)| a != b));
            checked += 1;
        }
        if checked == 0 {
            println!("no .2bpp/.1bpp files: run `make -C vendor/pokered` to give this test its oracle");
            return;
        }
        assert_eq!(checked, converted.len(), "some rgbgfx outputs were found but not all of them");
        let mut on_disk = Vec::new();
        walk(&Path::new(ROOT).join("gfx"), &mut on_disk);
        on_disk.retain(|f| f.extension().is_some_and(|e| e == "2bpp" || e == "1bpp"));
        assert_eq!(on_disk.len(), converted.len(), "make built an asset the pipeline does not");
        println!("{checked} assets match rgbgfx");
    }

    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { walk(&path, files) } else { files.push(path) }
        }
    }
}
