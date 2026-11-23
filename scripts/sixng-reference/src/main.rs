//! Thin validator over unmodified SixNG core, not a reimplementation.
use std::{error::Error,path::Path,time::Instant};
use sha2::{Digest,Sha256};
fn run()->Result<(),Box<dyn Error>>{
    let a:Vec<_>=std::env::args().collect();if a.len()<5{return Err("usage: reference range FILE START LEN | image FILE HASH MIP [SLICE]".into());}
    let store=sixng_core::ttpf::Ttpf::open(Path::new(&a[2]))?;
    let started=Instant::now();let mut info=serde_json::Value::Null;
    let (bytes,skipped)=match a[1].as_str(){
        // SixNG numbers its virtual payload space with an extra header page.
        // Accept neutral RDS5 addresses and adapt only that documented origin.
        "range"=>store.read_ex(a[3].parse::<u64>()?.checked_add(4096).ok_or("offset overflow")?,a[4].parse()?)?,
        "image"=>{let hash=u64::from_str_radix(a[3].trim_start_matches("0x"),16)?;let t=store.get(hash).ok_or("texture hash not found")?;let (data,skipped)=store.data_ex(t)?;let (rgba,_,_)=sixng_core::ttpf::decode_rgba_slice(t,&data,a[4].parse()?,a.get(5).map(|s|s.parse()).transpose()?.unwrap_or(0))?;info=serde_json::json!({"mips":t.mips(),"mips_fixed":t.mips_fixed,"size":t.size,"d0":t.d0,"d1":t.d1,"width":t.width,"height":t.height,"slices":t.slices()});(rgba,skipped)},
        _=>return Err("unknown command".into()),
    };
    let elapsed=started.elapsed().as_secs_f64();println!("{}",serde_json::json!({"bytes":bytes.len(),"skipped":skipped,"seconds":elapsed,"sha256":format!("{:x}",Sha256::digest(&bytes)),"texture":info}));Ok(())
}
fn main(){if let Err(e)=run(){eprintln!("reference: {e}");std::process::exit(1);}}
