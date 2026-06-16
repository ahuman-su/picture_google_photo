use std::{collections::btree_map::Entry, fs};
use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    input: String,

    #[arg(short, long, default_value="final/")]
    output: String,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();

    println!("chemin de input : {}", args.input);


    println!("chemin de outpute : {}", args.output);

    let noms: Vec<String> = list_name_file(args.input)?;

    for nom in noms{
        let is_picture:bool = pars_list_name_file(&nom);

        if is_picture {
            println!("is picture")
        }else {
            println!("is json")
        }
    }

    Ok(())


}


fn list_name_file(input: String) -> std::io::Result<Vec<String>> {
    //cherhce tout les noms de fichier du dossier input et les ressort dans un Vecteur
    let mut noms: Vec<String> = Vec::new();

    for entry in fs::read_dir(input)? {
        let entry = entry?;
        let file_name: std::ffi::OsString = entry.file_name();
        noms.push(file_name.to_string_lossy().to_string());
    }

    Ok(noms)
}

fn pars_list_name_file(nom: &str) -> bool{
    //return true si c'est une image et false si c'est un json
    if nom.ends_with(".json") {
        return false;
    }else {
        return true;
    }
}