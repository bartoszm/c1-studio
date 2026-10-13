//! Prints the size of each JPEG-XL file named on standard input, one per line, as
//! `<line>\t<width>\t<height>`, or `<line>\t-\t-` when it cannot be read.
use std::io::BufRead;

fn main() {
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let path = line.rsplit('\t').next().unwrap_or(&line);
        match c1image::jxl_size(std::path::Path::new(path)) {
            Ok((w, h)) => println!("{line}\t{w}\t{h}"),
            Err(_) => println!("{line}\t-\t-"),
        }
    }
}
