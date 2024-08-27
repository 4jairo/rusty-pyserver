use std::borrow::Cow;
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

const HTML_STYLE: &str = r#"
* {
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
}
button {
    cursor: pointer
}
div {
    margin: 20px 0;
    display: flex;
    gap: 20px;
}
body {
    width: 90%;
    margin: 10px auto;
}
table {
    border-collapse: collapse;
}
table th, table td {
    border: 1px solid black;
    padding: 5px 20px;
    text-align: center;
}
table th {
    background-color: #f1f1c1;
}
table th:last-child, table td:last-child {
    text-align: left;
}
table tr:nth-child(odd) {
    background-color: #d2d2d2;
}"#;

pub fn build_html2(uri_path: Cow<str>, files: Vec<DirectoryFile>) -> String {
    // let mut r = String::from("<head><title>Contents of ");
    // r.push_str(&uri_path);
    // r.push_str("</title><style>* {font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;}button {cursor: pointer}div {margin: 20px 0;display: flex;gap: 20px;}body {width: 90%;margin: 10px auto;}table {border-collapse: collapse;}table th, table td {border: 1px solid black;padding: 5px 20px;text-align: center;}table th {background-color: #f1f1c1;}table th:last-child, table td:last-child {text-align: left;}table tr:nth-child(odd) {background-color: #d2d2d2;}</style></head><body><h1>Contents of ");
    // r.push_str(&uri_path);
    // r.push_str(r#"</h1><div><button onclick="window.history.back()">Back</button><a href="/*"#);
    // r.push_str(&uri_path);
    // r.push_str(r#"">Download as ZIP from current path</a></div><table><thead><tr><th>Type</th><th>Size</th><th>File Name</th></tr></thead><tbody id="rows"></tbody></table><script type="text/javascript">const rows = "#);
    // r.push_str(&serde_json::to_string(&files).unwrap());
    // r.push_str(r#";let html = '';rows.forEach(([isDir, fileSize, fileName]) => {const icon = isDir ? '<svg width="1.2em" height="1.2em" viewBox="0 0 24 24"><path d="M4 20q-.825 0-1.412-.587T2 18V6q0-.825.588-1.412T4 4h6l2 2h8q.825 0 1.413.588T22 8H4v10l2.4-8h17.1l-2.575 8.575q-.2.65-.737 1.038T19 20z"/></svg>' : '<svg width="1.2em" height="1.2em" viewBox="0 0 24 24"><path d="M19 19H8q-.825 0-1.412-.587T6 17V3q0-.825.588-1.412T8 1h6.175q.4 0 .763.15t.637.425l4.85 4.85q.275.275.425.638t.15.762V17q0 .825-.587 1.413T19 19m0-11h-3.5q-.625 0-1.062-.437T14 6.5V3H8v14h11zM4 23q-.825 0-1.412-.587T2 21V8q0-.425.288-.712T3 7t.713.288T4 8v13h10q.425 0 .713.288T15 22t-.288.713T14 23zM8 3v5zv14z"/></svg>';html += `<tr><td>${icon}</td><td>${fileSize}</td><td><a href="${fileName}">${fileName}</a></td></tr>`;});document.getElementById('rows').innerHTML = html;</script></body>"#);
    // r

    let files_json = serde_json::to_string(&files).unwrap();
    
    format!(
r#"<head>
    <title>Contents of {uri_path}</title>
    <style>{HTML_STYLE}</style>
</head>
<body>
    <h1>Contents of {uri_path}</h1>
    <div>
        <button onclick="window.history.back()">Back</button>
        <a href="/*{uri_path}">Download as ZIP from current path</a>
    </div>

    <table>
        <thead>
            <tr>
                <th>Type</th>
                <th>Size</th>
                <th>File Name</th>
            </tr>
        </thead>
        <tbody id="rows"></tbody>
    </table>
    <script type="text/javascript">
        const rows = {files_json}
        let html = ''
        rows.forEach(([isDir, fileSize, fileName]) => {{
            const icon = isDir 
                ? '<svg width="1.2em" height="1.2em" viewBox="0 0 24 24"><path d="M4 20q-.825 0-1.412-.587T2 18V6q0-.825.588-1.412T4 4h6l2 2h8q.825 0 1.413.588T22 8H4v10l2.4-8h17.1l-2.575 8.575q-.2.65-.737 1.038T19 20z"/></svg>' 
                : '<svg width="1.2em" height="1.2em" viewBox="0 0 24 24"><path d="M19 19H8q-.825 0-1.412-.587T6 17V3q0-.825.588-1.412T8 1h6.175q.4 0 .763.15t.637.425l4.85 4.85q.275.275.425.638t.15.762V17q0 .825-.587 1.413T19 19m0-11h-3.5q-.625 0-1.062-.437T14 6.5V3H8v14h11zM4 23q-.825 0-1.412-.587T2 21V8q0-.425.288-.712T3 7t.713.288T4 8v13h10q.425 0 .713.288T15 22t-.288.713T14 23zM8 3v5zv14z"/></svg>'

            html += `<tr>
                <td>${{icon}}</td>
                <td>${{fileSize}}</td>
                <td><a href="${{fileName}}">${{fileName}}</a></td>
            </tr>`
        }});
        document.getElementById('rows').innerHTML = html;
    </script>
</body>"#
    )
}


/* 
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::{body::Frame, header::{CONTENT_TYPE, SERVER}, Response, StatusCode};
use tokio::{fs::ReadDir, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;
use crate::{BoxBodyResponse, SERVER_NAME_HEADER};
pub struct HtmlBuilder {
    stream: tokio::io::DuplexStream,
}
impl HtmlBuilder {
    pub async fn new<'a>(uri_path: &Cow<'a, str>) -> (Self, BoxBodyResponse) {
        let (a, b) = tokio::io::duplex(1024 * 8);

        let reader_stream = ReaderStream::new(b);
        let body = StreamBody::new(reader_stream.map_ok(Frame::data)).boxed();

        let response = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/html")
            .header(SERVER, SERVER_NAME_HEADER)
            .body(body)
            .unwrap();

        let mut builder = Self {
            stream: a,
        };

        builder.initial_html(uri_path).await;
        (builder, response)
    }

    pub async fn initial_html<'a>(&mut self, uri_path: &Cow<'a, str>) {
        let _ = self.stream.write_all(
            format!(
                r#"<head>
                    <title>Contents of {uri_path}</title>
                    <style>
                        * {{
                            font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, 'Open Sans', 'Helvetica Neue', sans-serif;
                        }}
                        button {{
                            cursor: pointer
                        }}
                        div {{
                            margin: 20px 0;
                            display: flex;
                            gap: 20px;
                        }}
                        body {{
                            width: 90%;
                            margin: 10px auto;
                        }}
                        table {{
                            border-collapse: collapse;
                        }}
                        table th, table td {{
                            border: 1px solid black;
                            padding: 5px 20px;
                            text-align: center;
                        }}
                        table th {{
                            background-color: #f1f1c1;
                        }}
                        table th:last-child, table td:last-child {{
                            text-align: left;
                        }}
                        table tr:nth-child(odd) {{
                            background-color: #d2d2d2;
                        }}            
                    </style>
                </head>
                <body>
                    <h1>Contents of {uri_path}</h1>
                    <div>
                        <button onclick="window.history.back()">Back</button>
                        <a href="/*{uri_path}">Download as ZIP from current path</a>
                    </div>
        
                    <table>
                        <tr>
                            <th>Type</th>
                            <th>Size</th>
                            <th>File Name</th>
                        </tr>"#,
            )
            .as_bytes(),
        ).await;
    }

    pub async fn close_html(&mut self) {
        let _ = self.stream.write_all(r#"</table></body>"#.as_bytes()).await;
    }

    pub async fn add_files_in_dir(&mut self, mut files_in_dir: ReadDir) {
        while let Ok(Some(e)) = files_in_dir.next_entry().await {
            let is_dir = match e.file_type().await {
                Ok(t) => t.is_dir(),
                Err(_) => false
            };

            let file_name = match is_dir {
                true => format!("{}/", e.path().file_name().unwrap().to_str().unwrap()),
                false => e.path().file_name().unwrap().to_str().unwrap().to_string()
            };

            let file_size = match is_dir {
                true => "".to_string(),
                false => format_file_size(
                    e.metadata().await.map(|m| m.len()).unwrap_or_default()
                )
            };

            let row = build_html_table_row(is_dir, file_size, file_name);
            let _ = self.stream.write_all(row.as_bytes()).await;
        }

        self.close_html().await;
    }
}
*/*/