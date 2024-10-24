/*
 * Hurl (https://hurl.dev)
 * Copyright (C) 2024 Orange
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *          http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 */
use std::collections::HashMap;
use std::path::Path;

use crate::http::core::*;
use crate::http::*;
use crate::util::path::ContextDir;

impl RequestSpec {
    /// Returns this request as curl arguments.
    /// It does not contain the requests cookies (they will be accessed from the client)
    pub fn curl_args(&self, context_dir: &ContextDir) -> Vec<String> {
        let mut arguments = vec![];

        let data =
            !self.multipart.is_empty() || !self.form.is_empty() || !self.body.bytes().is_empty();
        arguments.append(&mut self.method.curl_args(data));

        for header in self.headers.iter() {
            arguments.append(&mut header.curl_args());
        }

        let has_explicit_content_type = self.headers.contains_key(CONTENT_TYPE);
        if !has_explicit_content_type {
            if let Some(content_type) = &self.implicit_content_type {
                if content_type != "application/x-www-form-urlencoded"
                    && content_type != "multipart/form-data"
                {
                    arguments.push("--header".to_string());
                    arguments.push(format!("'{}: {content_type}'", CONTENT_TYPE));
                }
            } else if !self.body.bytes().is_empty() {
                match self.body {
                    Body::Text(_) => {
                        arguments.push("--header".to_string());
                        arguments.push(format!("'{}:'", CONTENT_TYPE));
                    }
                    Body::Binary(_) => {
                        arguments.push("--header".to_string());
                        arguments.push(format!("'{}: application/octet-stream'", CONTENT_TYPE));
                    }
                    Body::File(_, _) => {
                        arguments.push("--header".to_string());
                        arguments.push(format!("'{}:'", CONTENT_TYPE));
                    }
                }
            }
        }

        for param in self.form.iter() {
            arguments.push("--data".to_string());
            arguments.push(format!("'{}'", param.curl_arg_escape()));
        }
        for param in self.multipart.iter() {
            arguments.push("--form".to_string());
            arguments.push(format!("'{}'", param.curl_arg(context_dir)));
        }

        if !self.body.bytes().is_empty() {
            // See <https://curl.se/docs/manpage.html#-d> and <https://curl.se/docs/manpage.html#--data-binary>:
            //
            // > -d, --data <data>
            // > ...
            // > If you start the data with the letter @, the rest should be a file name to read the
            // > data from, or - if you want curl to read the data from stdin. Posting data from a
            // > file named 'foobar' would thus be done with -d, --data @foobar. When -d, --data is
            // > told to read from a file like that, carriage returns and newlines will be stripped
            // > out. If you do not want the @ character to have a special interpretation use
            // > --data-raw instead.
            // > ...
            // > --data-binary <data>
            // >
            // > (HTTP) This posts data exactly as specified with no extra processing whatsoever.
            //
            // In summary: if the payload is a file (@foo.bin), we must use --data-binary option in
            // order to curl to not process the data sent.
            let param = match self.body {
                Body::File(_, _) => "--data-binary",
                _ => "--data",
            };
            arguments.push(param.to_string());
            arguments.push(self.body.curl_arg(context_dir));
        }

        let querystring = if self.querystring.is_empty() {
            String::new()
        } else {
            let params = self
                .querystring
                .iter()
                .map(|p| p.curl_arg_escape())
                .collect::<Vec<String>>();
            params.join("&")
        };
        let url = if querystring.as_str() == "" {
            self.url.raw()
        } else if self.url.raw().contains('?') {
            format!("{}&{}", self.url.raw(), querystring)
        } else {
            format!("{}?{}", self.url.raw(), querystring)
        };
        arguments.push(format!("'{url}'"));

        arguments
    }
}

fn encode_byte(b: u8) -> String {
    format!("\\x{b:02x}")
}

fn encode_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| encode_byte(*b)).collect()
}

impl Method {
    pub fn curl_args(&self, data: bool) -> Vec<String> {
        match self.0.as_str() {
            "GET" => {
                if data {
                    vec!["--request".to_string(), "GET".to_string()]
                } else {
                    vec![]
                }
            }
            "HEAD" => vec!["--head".to_string()],
            "POST" => {
                if data {
                    vec![]
                } else {
                    vec!["--request".to_string(), "POST".to_string()]
                }
            }
            s => vec!["--request".to_string(), s.to_string()],
        }
    }
}

impl Header {
    pub fn curl_args(&self) -> Vec<String> {
        let name = &self.name;
        let value = &self.value;
        vec![
            "--header".to_string(),
            encode_shell_string(&format!("{name}: {value}")),
        ]
    }
}

impl Param {
    pub fn curl_arg_escape(&self) -> String {
        let name = &self.name;
        let value = escape_url(&self.value);
        format!("{name}={value}")
    }

    pub fn curl_arg(&self) -> String {
        let name = &self.name;
        let value = &self.value;
        format!("{name}={value}")
    }
}

impl MultipartParam {
    pub fn curl_arg(&self, context_dir: &ContextDir) -> String {
        match self {
            MultipartParam::Param(param) => param.curl_arg(),
            MultipartParam::FileParam(FileParam {
                name,
                filename,
                content_type,
                ..
            }) => {
                let path = context_dir.resolved_path(Path::new(filename));
                let value = format!("@{};type={}", path.to_string_lossy(), content_type);
                format!("{name}={value}")
            }
        }
    }
}

impl Body {
    pub fn curl_arg(&self, context_dir: &ContextDir) -> String {
        match self {
            Body::Text(s) => encode_shell_string(s),
            Body::Binary(bytes) => format!("$'{}'", encode_bytes(bytes)),
            Body::File(_, filename) => {
                let path = context_dir.resolved_path(Path::new(filename));
                format!("'@{}'", path.to_string_lossy())
            }
        }
    }
}

fn escape_url(s: &str) -> String {
    percent_encoding::percent_encode(s.as_bytes(), percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn encode_shell_string(s: &str) -> String {
    // $'...' form will be used to encode escaped sequence
    if escape_mode(s) {
        let escaped = escape_string(s);
        format!("$'{escaped}'")
    } else {
        format!("'{s}'")
    }
}

// the shell string must be in escaped mode ($'...')
// if it contains \n, \t or '
fn escape_mode(s: &str) -> bool {
    for c in s.chars() {
        if c == '\n' || c == '\t' || c == '\'' {
            return true;
        }
    }
    false
}

fn escape_string(s: &str) -> String {
    let mut escaped_sequences = HashMap::new();
    escaped_sequences.insert('\n', "\\n");
    escaped_sequences.insert('\t', "\\t");
    escaped_sequences.insert('\'', "\\'");
    escaped_sequences.insert('\\', "\\\\");

    let mut escaped = String::new();
    for c in s.chars() {
        match escaped_sequences.get(&c) {
            None => escaped.push(c),
            Some(escaped_seq) => escaped.push_str(escaped_seq),
        }
    }
    escaped
}

#[cfg(test)]
pub mod tests {
    use std::path::Path;
    use std::str::FromStr;

    use super::*;

    fn form_http_request() -> RequestSpec {
        let mut headers = HeaderVec::new();
        headers.push(Header::new(
            "Content-Type",
            "application/x-www-form-urlencoded",
        ));

        RequestSpec {
            method: Method("POST".to_string()),
            url: Url::from_str("http://localhost/form-params").unwrap(),
            headers,
            form: vec![
                Param {
                    name: String::from("param1"),
                    value: String::from("value1"),
                },
                Param {
                    name: String::from("param2"),
                    value: String::from("a b"),
                },
            ],
            implicit_content_type: Some("multipart/form-data".to_string()),
            ..Default::default()
        }
    }

    fn json_request() -> RequestSpec {
        let mut headers = HeaderVec::new();
        headers.push(Header::new("content-type", "application/vnd.api+json"));
        RequestSpec {
            method: Method("POST".to_string()),
            url: Url::from_str("http://localhost/json").unwrap(),
            headers,
            body: Body::Text("{\"foo\":\"bar\"}".to_string()),
            implicit_content_type: Some("application/json".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_encode_byte() {
        assert_eq!(encode_byte(1), "\\x01".to_string());
        assert_eq!(encode_byte(32), "\\x20".to_string());
    }

    #[test]
    fn method_curl_args() {
        assert!(Method("GET".to_string()).curl_args(false).is_empty());
        assert_eq!(
            Method("GET".to_string()).curl_args(true),
            vec!["--request".to_string(), "GET".to_string()]
        );

        assert_eq!(
            Method("POST".to_string()).curl_args(false),
            vec!["--request".to_string(), "POST".to_string()]
        );
        assert!(Method("POST".to_string()).curl_args(true).is_empty());

        assert_eq!(
            Method("PUT".to_string()).curl_args(false),
            vec!["--request".to_string(), "PUT".to_string()]
        );
        assert_eq!(
            Method("PUT".to_string()).curl_args(true),
            vec!["--request".to_string(), "PUT".to_string()]
        );
    }

    #[test]
    fn header_curl_args() {
        assert_eq!(
            Header::new("Host", "example.com").curl_args(),
            vec!["--header".to_string(), "'Host: example.com'".to_string()]
        );
        assert_eq!(
            Header::new("If-Match", "\"e0023aa4e\"").curl_args(),
            vec![
                "--header".to_string(),
                "'If-Match: \"e0023aa4e\"'".to_string()
            ]
        );
    }

    #[test]
    fn param_curl_args() {
        assert_eq!(
            Param {
                name: "param1".to_string(),
                value: "value1".to_string(),
            }
            .curl_arg(),
            "param1=value1".to_string()
        );
        assert_eq!(
            Param {
                name: "param2".to_string(),
                value: String::new(),
            }
            .curl_arg(),
            "param2=".to_string()
        );
        assert_eq!(
            Param {
                name: "param3".to_string(),
                value: "a=b".to_string(),
            }
            .curl_arg_escape(),
            "param3=a%3Db".to_string()
        );
        assert_eq!(
            Param {
                name: "param4".to_string(),
                value: "1,2,3".to_string(),
            }
            .curl_arg_escape(),
            "param4=1%2C2%2C3".to_string()
        );
    }

    #[test]
    fn requests_curl_args() {
        let context_dir = &ContextDir::default();
        assert_eq!(
            hello_http_request().curl_args(context_dir),
            vec!["'http://localhost:8000/hello'".to_string()]
        );
        assert_eq!(
            custom_http_request().curl_args(context_dir),
            vec![
                "--header".to_string(),
                "'User-Agent: iPhone'".to_string(),
                "--header".to_string(),
                "'Foo: Bar'".to_string(),
                "'http://localhost/custom'".to_string(),
            ]
        );
        assert_eq!(
            query_http_request().curl_args(context_dir),
            vec![
                "'http://localhost:8000/querystring-params?param1=value1&param2=a%20b'".to_string()
            ]
        );
        assert_eq!(
            form_http_request().curl_args(context_dir),
            vec![
                "--header".to_string(),
                "'Content-Type: application/x-www-form-urlencoded'".to_string(),
                "--data".to_string(),
                "'param1=value1'".to_string(),
                "--data".to_string(),
                "'param2=a%20b'".to_string(),
                "'http://localhost/form-params'".to_string(),
            ]
        );
        assert_eq!(
            json_request().curl_args(context_dir),
            vec![
                "--header".to_string(),
                "'content-type: application/vnd.api+json'".to_string(),
                "--data".to_string(),
                "'{\"foo\":\"bar\"}'".to_string(),
                "'http://localhost/json'".to_string(),
            ]
        );

        assert_eq!(
            RequestSpec {
                method: Method("GET".to_string()),
                url: Url::from_str("http://localhost:8000/").unwrap(),
                ..Default::default()
            }
            .curl_args(context_dir),
            vec!["'http://localhost:8000/'".to_string(),]
        );
    }

    #[test]
    fn post_data_curl_args() {
        let context_dir = &ContextDir::default();
        let req = RequestSpec {
            method: Method("POST".to_string()),
            url: Url::from_str("http://localhost:8000/hello").unwrap(),
            body: Body::Text("foo".to_string()),
            ..Default::default()
        };
        assert_eq!(
            req.curl_args(context_dir),
            vec![
                "--header",
                "'Content-Type:'",
                "--data",
                "'foo'",
                "'http://localhost:8000/hello'"
            ]
        );

        let context_dir = &ContextDir::default();
        let req = RequestSpec {
            method: Method("POST".to_string()),
            url: Url::from_str("http://localhost:8000/hello").unwrap(),
            body: Body::File(b"Hello World!".to_vec(), "foo.bin".to_string()),
            ..Default::default()
        };
        assert_eq!(
            req.curl_args(context_dir),
            vec![
                "--header",
                "'Content-Type:'",
                "--data-binary",
                "'@foo.bin'",
                "'http://localhost:8000/hello'"
            ]
        );
    }

    #[test]
    fn test_encode_body() {
        let current_dir = Path::new("/tmp");
        let file_root = Path::new("/tmp");
        let context_dir = ContextDir::new(current_dir, file_root);
        assert_eq!(
            Body::Text("hello".to_string()).curl_arg(&context_dir),
            "'hello'".to_string()
        );

        if cfg!(unix) {
            assert_eq!(
                Body::File(vec![], "filename".to_string()).curl_arg(&context_dir),
                "'@/tmp/filename'".to_string()
            );
        }

        assert_eq!(
            Body::Binary(vec![1, 2, 3]).curl_arg(&context_dir),
            "$'\\x01\\x02\\x03'".to_string()
        );
    }

    #[test]
    fn test_encode_shell_string() {
        assert_eq!(encode_shell_string("hello"), "'hello'");
        assert_eq!(encode_shell_string("\\n"), "'\\n'");
        assert_eq!(encode_shell_string("'"), "$'\\''");
        assert_eq!(encode_shell_string("\\'"), "$'\\\\\\''");
        assert_eq!(encode_shell_string("\n"), "$'\\n'");
    }

    #[test]
    fn test_escape_string() {
        assert_eq!(escape_string("hello"), "hello");
        assert_eq!(escape_string("\\n"), "\\\\n");
        assert_eq!(escape_string("'"), "\\'");
        assert_eq!(escape_string("\\'"), "\\\\\\'");
        assert_eq!(escape_string("\n"), "\\n");
    }

    #[test]
    fn test_escape_mode() {
        assert!(!escape_mode("hello"));
        assert!(!escape_mode("\\"));
        assert!(escape_mode("'"));
        assert!(escape_mode("\n"));
    }
}
