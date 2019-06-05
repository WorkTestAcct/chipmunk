use crate::chunks::serialize_chunks;
use crate::parse::line_matching_format_expression;
use crate::parse::{match_format_string_in_file, read_format_string_options, FormatTestOptions};
#[macro_use]
extern crate clap;
use clap::{App, Arg, SubCommand};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

mod chunks;
mod merger;
mod parse;
mod processor;

mod timedline;
mod utils;

fn main() {
    let start = Instant::now();
    let matches = App::new("logviewer_parser")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Create index file and mapping file for logviewer")
        .arg(
            Arg::with_name("v")
                .short("v")
                .multiple(true)
                .help("Sets the level of verbosity"),
        )
        .subcommand(
            SubCommand::with_name("index")
                .about("command for creating an index file")
                .arg(
                    Arg::with_name("input")
                        .short("i")
                        .long("input")
                        .help("Sets the input file to be indexed")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::with_name("tag")
                        .short("t")
                        .long("tag")
                        .value_name("TAG")
                        .help("tag for each log entry")
                        .required(true),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("out")
                        .value_name("OUT")
                        .help("Output file, \"<file_to_index>.out\" if not present"),
                )
                .arg(
                    Arg::with_name("max_lines")
                        .short("n")
                        .long("max_lines")
                        .help("How many lines to collect before dumping")
                        .required(false)
                        .default_value("1000000"),
                )
                .arg(
                    Arg::with_name("chunk_size")
                        .short("c")
                        .long("chunk_siz")
                        .help("How many lines should be in a chunk (used for access later)")
                        .required(false)
                        .default_value("500"),
                )
                .arg(
                    Arg::with_name("append")
                        .short("a")
                        .long("append")
                        .help("append to file if exists"),
                )
                .arg(
                    Arg::with_name("stdout")
                        .short("s")
                        .long("stdout")
                        .help("put out chunk information on stdout"),
                ),
        )
        .subcommand(
            SubCommand::with_name("merge")
                .about("command for merging multiple log files")
                .arg(
                    Arg::with_name("merge_config")
                        .short("m")
                        .long("merge")
                        .help("input file is a json file that defines all files to be merged")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("out")
                        .value_name("OUT")
                        .required(true)
                        .help("Output file"),
                )
                .arg(
                    Arg::with_name("max_lines")
                        .short("n")
                        .long("max_lines")
                        .help("How many lines to collect before dumping")
                        .required(false)
                        .default_value("1000000"),
                )
                .arg(
                    Arg::with_name("chunk_size")
                        .short("c")
                        .long("chunk_siz")
                        .help("How many lines should be in a chunk (used for access later)")
                        .required(false)
                        .default_value("500"),
                )
                .arg(
                    Arg::with_name("append")
                        .short("a")
                        .long("append")
                        .help("append to file if exists"),
                )
                .arg(
                    Arg::with_name("stdout")
                        .short("s")
                        .long("stdout")
                        .help("put out chunk information on stdout"),
                ),
        )
        .subcommand(
            SubCommand::with_name("format")
                .about("test format string")
                .arg(
                    Arg::with_name("format-string")
                        .short("f")
                        .help("format string to use")
                        .long("format")
                        .requires("test-string")
                        .value_name("FORMAT_STR")
                        .required(false),
                )
                .arg(
                    Arg::with_name("test-string")
                        .short("t")
                        .long("test")
                        .requires("format-string")
                        .help("test string to use")
                        .value_name("SAMPLE")
                        .required(false),
                )
                .arg(
                    Arg::with_name("test-config")
                        .short("c")
                        .long("config")
                        .help("test a file using this configuration")
                        .value_name("CONFIG")
                        .required(false),
                ),
        )
        .get_matches();

    // Vary the output based on how many times the user used the "verbose" flag
    // (i.e. 'myprog -v -v -v' or 'myprog -vvv' vs 'myprog -v'
    // match matches.occurrences_of("v") {
    //     0 => println!("No verbose info"),
    //     1 => println!("Some verbose info"),
    //     2 => println!("Tons of verbose info"),
    //     3 | _ => println!("Don't be crazy"),
    // }

    if let Some(matches) = matches.subcommand_matches("merge") {
        if matches.is_present("merge_config") {
            let merge_config_file_name: &str = matches
                .value_of("merge_config")
                .expect("merge_config must be present");
            let out_path: PathBuf = match matches.value_of("output") {
                Some(path) => PathBuf::from(path),
                None => {
                    eprintln!("no output file specified");
                    process::exit(2)
                }
            };
            let max_lines = value_t_or_exit!(matches.value_of("max_lines"), usize);
            let chunk_size = value_t_or_exit!(matches.value_of("chunk_size"), usize);
            let append: bool = matches.is_present("append");
            let stdout: bool = matches.is_present("stdout");
            let merger = merger::Merger {
                max_lines,  // how many lines to collect before writing out
                chunk_size, // used for mapping line numbers to byte positions
            };
            let config_path = PathBuf::from(merge_config_file_name);
            let merged_lines =
                match merger.merge_files_use_config_file(&config_path, &out_path, append, stdout) {
                    Ok(cnt) => cnt,
                    Err(e) => {
                        eprintln!("error merging: {}", e);
                        process::exit(2)
                    }
                };
            let elapsed = start.elapsed();
            let ms = elapsed.as_millis();
            let duration_in_s = ms as f64 / 1000.0;
            eprintln!("merging {} lines took {:.3}s!", merged_lines, duration_in_s);
        }
    }
    if let Some(matches) = matches.subcommand_matches("index") {
        if matches.is_present("input") && matches.is_present("tag") {
            let file = matches.value_of("input").expect("input must be present");
            let tag = matches.value_of("tag").expect("tag must be present");
            let fallback_out = file.to_string() + ".out";
            let out_path = PathBuf::from(
                matches
                    .value_of("output")
                    .unwrap_or_else(|| fallback_out.as_str()),
            );
            let mapping_out_path: PathBuf = PathBuf::from(file.to_string() + ".map.json");
            let max_lines = value_t_or_exit!(matches.value_of("max_lines"), usize);
            let chunk_size = value_t_or_exit!(matches.value_of("chunk_size"), usize);
            let indexer = processor::Indexer {
                source_id: tag.to_string(), // tag to append to each line
                max_lines,                  // how many lines to collect before writing out
                chunk_size,                 // used for mapping line numbers to byte positions
            };

            let f = match fs::File::open(&file) {
                Ok(file) => file,
                Err(_) => {
                    eprintln!("could not open {}", file);
                    process::exit(2)
                }
            };

            let source_file_size = match f.metadata() {
                Ok(file_meta) => file_meta.len() as usize,
                Err(_) => {
                    eprintln!("could not find out size of source file");
                    process::exit(2);
                }
            };
            let append: bool = matches.is_present("append");
            let stdout: bool = matches.is_present("stdout");
            match indexer.index_file(&f, &out_path, append, source_file_size, stdout) {
                Err(why) => {
                    eprintln!("couldn't process: {}", why);
                    process::exit(2)
                }
                Ok(chunks) => {
                    let _ = serialize_chunks(&chunks, &mapping_out_path);
                    let elapsed = start.elapsed();
                    let ms = elapsed.as_millis();
                    let duration_in_s = ms as f64 / 1000.0;
                    let file_size_in_mb = source_file_size as f64 / 1024.0 / 1024.0;
                    let mb_bytes_per_second: f64 = file_size_in_mb / duration_in_s;
                    eprintln!(
                        "processing ~{} MB took {:.3}s! ({:.3} MB/s)",
                        file_size_in_mb.round(),
                        duration_in_s,
                        mb_bytes_per_second
                    );
                }
            }
        }
    }
    if let Some(matches) = matches.subcommand_matches("format") {
        if matches.is_present("test-string") && matches.is_present("format-string") {
            let format_string = matches
                .value_of("format-string")
                .expect("format-string must be present");
            let test_string = matches
                .value_of("test-string")
                .expect("test-string must be present");
            println!(
                "format-string: {}, test_string: {}",
                format_string, test_string
            );
            match line_matching_format_expression(format_string, test_string) {
                Ok(res) => println!("match: {:?}", res),
                Err(e) => {
                    eprintln!("error matching: {}", e);
                    process::exit(2)
                }
            }
        } else if matches.is_present("test-config") {
            let test_config_name = matches
                .value_of("test-config")
                .expect("test-config-name must be present");
            let config_path = PathBuf::from(test_config_name);
            // match_format_string_in_file(format_expr: &str, file_name: &str)
            let mut test_config_file = match fs::File::open(&config_path) {
                Ok(file) => file,
                Err(_) => {
                    eprintln!("could not open {}", test_config_name);
                    process::exit(2)
                }
            };
            let options: FormatTestOptions = match read_format_string_options(&mut test_config_file)
            {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("could not parse format config file: {}", e);
                    process::exit(2)
                }
            };
            match match_format_string_in_file(
                options.format.as_str(),
                options.file.as_str(),
                options.lines_to_test,
            ) {
                Ok(res) => match serde_json::to_string(&res) {
                    Ok(json) => eprintln!("{}", json),
                    Err(e) => {
                        eprintln!("serializing result failed: {}", e);
                        process::exit(2)
                    }
                },
                Err(e) => {
                    eprintln!("could not match format string file: {}", e);
                    process::exit(2)
                }
            }
        }
    }
}