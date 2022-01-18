use std::{collections::HashMap, path::Path};

use serde::Serialize;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

use async_recursion::async_recursion;
//use git2::Repository;
use lazy_static::lazy_static;
//use rayon::prelude::*;
use regex::Regex;
//use tempdir::TempDir;

const HTTPS_REPO_URL: &'static str = "https://github.com/dotnet/runtime";
const URL_REGEX_CONTENT: &'static str = r#"https?://(www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_\+.~#?&//=]*)"#;

const GITHUB_API_KEY: &'static str = "--> INSERT YOUR API KEY <--";

#[cfg(debug_assertions)]
lazy_static! {
    static ref URL_REGEX: Regex = Regex::new(URL_REGEX_CONTENT).unwrap();
}

#[cfg(not(debug_assertions))]
lazy_static! {
    static ref URL_REGEX: Regex = unsafe { Regex::new(URL_REGEX_CONTENT).unwrap_unchecked() };
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create temporary directory containing the current runtime
    //let temp_dir = TempDir::new("dotnet-runtime").expect("Failed to create temporary directory");
    //let temp_dir_path = temp_dir.path();

    let temp_dir_path = Path::new("E:\\external\\dotnet\\runtime");

    println!(
        "Cloning {} into temporary directory {}",
        HTTPS_REPO_URL,
        temp_dir_path.display()
    );
    //Repository::clone(HTTPS_REPO_URL, temp_dir_path).expect("Cloning failed");

    let found_urls = iterate_recursive(temp_dir_path)
        .await
        .expect("Failed iterating");

    println!("-- Mapping code parts to issues --");
    let urls_len = found_urls.len();

    let mut issue_map = HashMap::<u32, Vec<Found>>::new();
    for (index, found) in found_urls.iter().enumerate() {
        // Filter
        let lower_snippet = found.snippet.to_lowercase();
        if lower_snippet.contains("[ActiveIssue(")
            || lower_snippet.contains("fix")
            || lower_snippet.contains("resolve")
            || lower_snippet.contains("workaround")
            || lower_snippet.contains("work around")
            || lower_snippet.contains("for now")
            || lower_snippet.contains("temporarily")
            || lower_snippet.contains("currently")
        {
            let issue_id = extract_issue_id(found.url.clone());
            let issue_map_entry = issue_map.entry(issue_id).or_insert(Vec::new());
            issue_map_entry.push(found.clone());
            println!("[{}/{}] Mapping {}", index, urls_len, issue_id);
        } else {
            println!("[{}/{}] Skipping", index, urls_len);
        }
    }

    println!("-- Checking issues (only collect closed issues) --");
    let octocrab = octocrab::OctocrabBuilder::new()
        .personal_token(GITHUB_API_KEY.to_string())
        .build()
        .unwrap();

    let overall_count = issue_map.len();
    let issue_handler = octocrab.issues("dotnet", "runtime");
    for (index, issue_id) in issue_map.clone().keys().enumerate() {
        let issue = issue_handler.get(*issue_id as u64).await;
        if issue.is_err() {
            continue;
        }
        if issue.unwrap().closed_at.is_none() {
            // Open issue, remove from map
            println!(
                "[{}/{}] Found open issue {}",
                index, overall_count, issue_id
            );
            issue_map.remove(issue_id);
            continue;
        }
        println!(
            "[{}/{}] Found closed issue {}",
            index, overall_count, issue_id
        );
    }

    println!("-- Sort issues --");
    let mut sorted_issues: Vec<_> = issue_map.iter().collect();
    sorted_issues.sort_by_key(|(x, _)| *x);

    let results = serde_json::to_string_pretty(&issue_map).expect("Failed to serialize");
    println!("-- Generating results.json --");
    tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open("results.json")
        .await
        .expect("Failed to open results.json")
        .write_all(results.as_bytes())
        .await
        .expect("Failed to write results.json");

    println!("-- Generating results.md --");

    let mut result_md = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open("results.md")
        .await
        .expect("Failed to open results.md");

    result_md
        .write_all(b"# Find Closed Issues Tool Results\n")
        .await
        .expect("Failed to write results.md");
    for (key, value) in sorted_issues.iter() {
        result_md
            .write_all(
                format!(
                    "\n## [Issue {}](https://github.com/dotnet/runtime/issues/{})\n",
                    *key, *key
                )
                .as_bytes(),
            )
            .await
            .expect("Failed to write results.md");

        for found in value.iter() {
            result_md
                .write_all(
                    format!(
                        "\nFile `{}` (File Position {}-{})\n\n{}/{}#L{}-L{}\n",
                        found.file, found.start, found.end,
                        "https://github.com/dotnet/runtime/blob/f04a24249835096eea1a1a66e4af03cfec5ed32b",
                        found.file,
                        found.line - 2,
                        found.line + 2
                    )
                    .as_bytes(),
                )
                .await
                .expect("Failed to write results.md");
        }
    }

    println!("-- Done --");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::extract_issue_id;
    use super::find_line_of_position;

    #[test]
    fn test_extract_last_issue_id() {
        let input = "https://github.com/dotnet/runtime/issues/123";
        let expected = 123;
        let actual = extract_issue_id(input.to_string());

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_extract_last_issue_id_with_special_char() {
        let input = "https://github.com/dotnet/runtime/issues/123).#123";
        let expected = 123;
        let actual = extract_issue_id(input.to_string());

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_find_line_of_position() {
        let code = r#"Line 1
                           Line 2
                           Line 3"#;
        let pos = 8;
        let expected = 2;
        let actual = find_line_of_position(code, pos).unwrap();

        assert_eq!(actual, expected);
    }
}

fn extract_issue_id(url: String) -> u32 {
    let mut u = url.clone();

    // Remove fragments
    if let Some(fragment_start_pos) = url.rfind('#') {
        u = u[..fragment_start_pos].to_string();
    }

    // Remove query parameters
    if let Some(fragment_start_pos) = url.rfind('?') {
        u = u[..fragment_start_pos].to_string();
    }

    while !(u.ends_with('0')
        || u.ends_with('1')
        || u.ends_with('2')
        || u.ends_with('3')
        || u.ends_with('4')
        || u.ends_with('5')
        || u.ends_with('6')
        || u.ends_with('7')
        || u.ends_with('8')
        || u.ends_with('9'))
    {
        u.pop();
    }

    let last_slash_pos = url.rfind('/').unwrap();
    u[last_slash_pos + 1..].parse::<u32>().unwrap_or_else(|_| 0)
}

#[async_recursion]
async fn iterate_recursive<P: AsRef<Path> + Send>(
    path: P,
) -> Result<Vec<Found>, Box<dyn std::error::Error>> {
    let mut result = vec![];
    let mut read_result = tokio::fs::read_dir(path)
        .await
        .expect("Failed to read directory");

    while let Ok(opt) = read_result.next_entry().await {
        match opt {
            Some(entry) => {
                if let Ok(metadata) = tokio::fs::metadata(entry.path()).await {
                    if metadata.is_dir() {
                        let file_name_opt = entry.file_name();
                        let file_name = file_name_opt.to_str().expect("Failed to parse file name");
                        if file_name.starts_with(".") || file_name == "artifacts" {
                            continue;
                        }
                        let sub_vec = iterate_recursive(entry.path()).await?;
                        for sub_entry in sub_vec {
                            if result.contains(&sub_entry) {
                                continue;
                            }
                            result.push(sub_entry);
                        }
                    } else {
                        let file_name_opt = entry.file_name();
                        let file_name = file_name_opt.to_str().expect("Failed to parse file name");

                        if file_name.ends_with(".dll")
                            || file_name.ends_with(".pdb")
                            || file_name.ends_with(".exe")
                            || file_name.ends_with(".lib")
                        {
                            continue;
                        }

                        let p = entry.path();
                        let s = format!("{}", p.display());
                        for url in get_urls(p).await.expect("Cannot open file") {
                            println!(
                                "Found URL {} in File {} (pos {} - {})",
                                url.url, s, url.start, url.end
                            );
                            if !result.contains(&url) {
                                result.push(url);
                            }
                        }
                    }
                }
            }
            None => break,
        }
    }

    Ok(result)
}

#[derive(Debug, PartialEq, PartialOrd, Eq, Ord, Clone, Hash, Serialize)]
struct Found {
    file: String,
    url: String,
    snippet: String,
    line: usize,
    start: usize,
    end: usize,
}

async fn get_urls<P: AsRef<Path> + Clone>(
    path: P,
) -> Result<Vec<Found>, Box<dyn std::error::Error>> {
    let mut occurrences = vec![];
    let mut file = File::open(path.clone()).await?;

    let mut contents = Vec::<u8>::new();
    file.read_to_end(&mut contents).await?;

    let text_result = String::from_utf8(contents);
    if text_result.is_err() {
        return Ok(occurrences);
    }

    let text = text_result.unwrap();

    let mut capture_locations = URL_REGEX.capture_locations();
    let match_result = URL_REGEX.captures_read(&mut capture_locations, &text);
    if match_result.is_none() {
        return Ok(occurrences);
    }

    let m = match_result.unwrap();
    let url = m.as_str().to_string();
    let start = m.start();
    let end = m.end();

    let line = find_line_of_position(&text, start).expect("Line not found");
    let snippet_lines = text.lines().skip(line - 2).take(2).collect::<Vec<&str>>();
    let snippet = snippet_lines.join("\n");

    if url.contains("github.com/dotnet/runtime/issues") {
        occurrences.push(Found {
            file: format!("{}", path.as_ref().display())
                .replace("E:\\external\\dotnet\\runtime\\", "")
                .replace("\\", "/"),
            url,
            start,
            line,
            snippet,
            end,
        });
    }

    Ok(occurrences)
}

fn find_line_of_position(s: &str, pos: usize) -> Option<usize> {
    let mut line_number = 0usize;
    let mut current_pos = 0usize;

    // We do not use s.lines() because it also splits \r characters
    // and we want to count the newline character too
    for line in s.split('\n') {
        line_number += 1;
        current_pos += 1; // \n character
        current_pos += line.bytes().count();
        if current_pos >= pos {
            return Some(line_number);
        }
    }
    None
}
