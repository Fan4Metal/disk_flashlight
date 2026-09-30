//! CSV export of the Largest files and Search lists, from their tabs and
//! with `--export`.

use std::io::{self, Write};
use std::path::Path;

use crate::format::date_time_at;
use crate::model::Model;
use crate::scan::walk::ScanError;

/// Column names, in English so that scripts do not depend on the interface
/// language.
const HEADER: [&str; 5] = ["path", "type", "size", "allocated", "modified"];
/// Columns of the folder tree: `level` is the depth below the scan root.
const TREE_HEADER: [&str; 7] = ["path", "level", "size", "allocated", "files", "folders", "modified"];

/// CSV lines with `sep` between fields: UTF-8 with a BOM (Excel needs it
/// for non-ASCII paths), CRLF line ends and quoting as in RFC 4180.
struct CsvWriter<'a, W: Write + ?Sized> {
    w: &'a mut W,
    sep: char,
    line: String,
}

impl<'a, W: Write + ?Sized> CsvWriter<'a, W> {
    fn new(w: &'a mut W, sep: char, header: &[&str]) -> io::Result<Self> {
        w.write_all("\u{feff}".as_bytes())?;
        let mut csv = Self { w, sep, line: String::new() };
        csv.row(header)?;
        Ok(csv)
    }

    fn row(&mut self, fields: &[&str]) -> io::Result<()> {
        self.line.clear();
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                self.line.push(self.sep);
            }
            push_field(&mut self.line, f, self.sep);
        }
        self.line.push_str("\r\n");
        self.w.write_all(self.line.as_bytes())
    }
}

/// Write `ids` as CSV. Sizes are in bytes; times are local (`offset`
/// seconds from UTC), empty when unknown.
pub fn write_items(w: &mut (impl Write + ?Sized), model: &Model, ids: &[u32], sep: char, offset: i64) -> io::Result<()> {
    let mut csv = CsvWriter::new(w, sep, &HEADER)?;
    for &id in ids {
        let n = model.node(id);
        let kind = if n.is_dir { "folder" } else { "file" };
        let (size, alloc) = (n.size.to_string(), n.alloc.to_string());
        let modified = date_time_at(n.modified, offset);
        csv.row(&[&model.path(id), kind, &size, &alloc, &modified])?;
    }
    Ok(())
}

/// Write the folder tree rows `(id, level)` as CSV, with the number of
/// files and folders inside each; as [`write_items`] otherwise.
pub fn write_tree(w: &mut (impl Write + ?Sized), model: &Model, rows: &[(u32, u16)], sep: char, offset: i64) -> io::Result<()> {
    let mut csv = CsvWriter::new(w, sep, &TREE_HEADER)?;
    for &(id, level) in rows {
        let n = model.node(id);
        let numbers = [level.to_string(), n.size.to_string(), n.alloc.to_string(), n.files.to_string(), n.dirs.to_string()];
        let [level, size, alloc, files, dirs] = numbers.each_ref().map(String::as_str);
        let modified = date_time_at(n.modified, offset);
        csv.row(&[&model.path(id), level, size, alloc, files, dirs, &modified])?;
    }
    Ok(())
}

/// Write the folders a scan could not read, with the system's message.
pub fn write_errors(w: &mut (impl Write + ?Sized), errors: &[ScanError], sep: char) -> io::Result<()> {
    let mut csv = CsvWriter::new(w, sep, &["path", "error"])?;
    for e in errors {
        csv.row(&[&e.path, &e.message])?;
    }
    Ok(())
}

/// Create the file at `path` and `write` into it, with local times.
pub fn save(path: &Path, write: impl FnOnce(&mut dyn Write, i64) -> io::Result<()>) -> io::Result<()> {
    let mut w = io::BufWriter::new(std::fs::File::create(path)?);
    write(&mut w, crate::format::local_offset())?;
    w.flush()
}

/// `text` as a field, quoted when it holds the separator, a quote or a
/// line break, with quotes doubled.
fn push_field(out: &mut String, text: &str, sep: char) {
    if text.contains([sep, '"', '\r', '\n']) {
        out.push('"');
        out.push_str(&text.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RawDir, RawFile};

    #[test]
    fn writes_errors() {
        let e = |path: &str, message: &str| ScanError { path: path.into(), message: message.into() };
        let errors = [e(r"C:\Windows\CSC", "Access is denied. (os error 5)"), e(r"C:\a,b", "x")];
        let mut out = Vec::new();
        write_errors(&mut out, &errors, ',').unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\u{feff}path,error\r\nC:\\Windows\\CSC,Access is denied. (os error 5)\r\n\"C:\\a,b\",x\r\n"
        );
    }

    #[test]
    fn writes_quoted_rows() {
        let f = |name: &str, size: u64, modified: u32| RawFile { name: name.into(), size, alloc: 4096, modified };
        let raw = RawDir {
            name: "root".into(),
            files: vec![f("a,b.txt", 10, 1_710_511_331), f("say \"hi\";.txt", 5, 0)],
            subdirs: vec![RawDir { name: "Фото".into(), files: vec![f("x", 1, 0)], ..Default::default() }],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let id = |name: &str| (0..m.len() as u32).find(|&i| m.name(i) == name).unwrap();
        let ids = [id("a,b.txt"), id("say \"hi\";.txt"), id("Фото")];
        let text = |sep| {
            let mut out = Vec::new();
            write_items(&mut out, &m, &ids, sep, 0).unwrap();
            String::from_utf8(out).unwrap()
        };
        let comma = text(',');
        let (a, b, photo) = (m.path(ids[0]), m.path(ids[1]), m.path(ids[2]));
        assert_eq!(
            comma,
            format!(
                "\u{feff}path,type,size,allocated,modified\r\n\
                 \"{a}\",file,10,4096,2024-03-15 14:02:11\r\n\
                 \"{}\",file,5,4096,\r\n\
                 {photo},folder,1,4096,\r\n",
                b.replace('"', "\"\"")
            )
        );
        // The tree: levels and counts inside each folder.
        let mut out = Vec::new();
        write_tree(&mut out, &m, &[(0, 0), (ids[2], 1)], ',', 0).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!(
                "\u{feff}path,level,size,allocated,files,folders,modified\r\n\
                 {},0,16,12288,3,1,2024-03-15 14:02:11\r\n\
                 {photo},1,1,4096,1,0,\r\n",
                m.path(0)
            )
        );
        // With `;` the comma needs no quotes, the semicolon does.
        let semi = text(';');
        assert!(semi.contains(&format!("\r\n{a};file;10;")), "{semi}");
        assert!(semi.contains(&format!("\r\n\"{}\";file;5;", b.replace('"', "\"\""))), "{semi}");
    }
}
