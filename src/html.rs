use std::borrow::Cow;
use askama::Template;
use serde::{ser::SerializeTuple, Serialize, Serializer};


pub fn format_file_size(file_size: u64) -> String {
    if file_size < 1000 {
        return format!("{} B", file_size);
    }
 
    let kb = file_size as f64 / 1000.0;
    if kb < 1000.0 {
        return format!("{:.2} KB", kb);
    }

    let mb = kb / 1000.0;
    if mb < 1000.0 {
        return format!("{:.2} MB", mb);
    }

    let gb = mb / 1000.0;
    format!("{:.2} GB", gb)
}

pub struct DirectoryFile {
    pub is_dir: bool,
    pub file_size: String,
    pub file_name: String,
}
impl Serialize for DirectoryFile {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple = serializer.serialize_tuple(3)?; 
        tuple.serialize_element(&self.is_dir)?;
        tuple.serialize_element(&self.file_size)?;
        tuple.serialize_element(&self.file_name)?;
        tuple.end()
    }
}


#[derive(Template)]
#[template(path = "index.html", escape = "none")]
pub struct HtmlTemplate<'a> {
    uri_path: Cow<'a, str>,
    files_json: String
}
impl<'a> HtmlTemplate<'a> {
    pub fn new(uri_path: Cow<'a, str>, files: Vec<DirectoryFile>) ->  serde_json::error::Result<Self> {
        Ok(Self { 
            uri_path, 
            files_json: serde_json::to_string(&files)?
        })
    }
}