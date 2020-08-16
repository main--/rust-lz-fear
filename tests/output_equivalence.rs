use lz_fear::framed::CompressionSettings;
use std::env;
use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

fn run_cmd(flags: &[&str]) -> Vec<u8> {
    let me = env::current_exe().unwrap();
    let mut cmd = Command::new("lz4");
    cmd.args(flags);
    cmd.arg(me);

//    println!("running {:?}", cmd);
    let output = cmd.output().unwrap();

    assert!(output.status.success());
    output.stdout
}

// content checksum
// block dependency
// changed blocksize
// dictionary
// contentsize or not

/*
#[test]
fn test() {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let file_in = File::open(filename_in)?;
    let file_out = File::create(filename_out)?;
    
    CompressionSettings::default()
        .content_checksum(true)
        .independent_blocks(true)
        /*.block_size(64 * 1024).dictionary(0, &vec![0u8; 64 * 1024]).dictionary_id_nonsense_override(Some(42))*/
        .compress_with_size(file_in, file_out)?;
}
*/

//static FLAGS = [""];

static DICT_DATA: &'static [u8] = &[1,3,3,7];

#[test]
fn run_test() {
    let mut failed_runs = Vec::new();

    let dict_data = DICT_DATA;
    let dict_data_file = {
        let mut f = NamedTempFile::new().expect("Error creating temporary file");
        f.write_all(dict_data).expect("Error writing DICT_DATA");
        f
    };
    let dict_data_path = dict_data_file.path().to_str().unwrap();

    for bits in 0..(1 << 5) {
        let mut settings = CompressionSettings::default();
        let mut args = Vec::new();

        if bits & 1 != 0 {
            settings.content_checksum(false);
            args.push("--no-frame-crc");
        }

        if bits & 2 != 0 {
            settings.independent_blocks(false);
            args.push("-BD");
        }

        if bits & 4 != 0 {
            continue;
            settings.block_size(256 * 1024);
            args.push("-B5");
        }

        if bits & 8 != 0 {
            settings.dictionary(0, dict_data).dictionary_id_nonsense_override(None);
            args.extend(&["-D", dict_data_path]);
        }

        let input = std::fs::File::open(env::current_exe().unwrap()).unwrap(); //std::io::Cursor::new(&[1,3,3,7]);
        let mut output = Vec::new();
        if bits & 16 != 0 {
            settings.compress_with_size(input, &mut output);
            args.push("--content-size");
        } else {
            settings.compress(input, &mut output);
        }
        
        let reference_output = run_cmd(&args);
        if !output.iter().copied().eq(reference_output) {
            println!("fail={:?}", args);
            failed_runs.push(args);
        }
        //println!("{:x?} vs {:x?}", reference_output, output);
    }
    assert!(failed_runs.is_empty());
}

