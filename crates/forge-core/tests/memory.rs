use forge_core::memory::*;
use serde_json::json;
#[test]
fn parses_valid_memory_parts_and_discards_malformed_parts(){let m=parse_mem(&json!({"t":"mem","peak":5000,"samples":[[0.1,1000],[0.2,"x"],[0.3,3000]],"sites":[["app.py",6,4000,20],["bad"]],"types":[["app.Item",20],[3,4]],"final":true,"kept":[{"type":"app.Item","count":20,"path":["app.CACHE","list"]},{"type":1}]})).unwrap();assert_eq!(m.samples,vec![(0.1,1000.),(0.3,3000.)]);assert_eq!(m.sites.as_ref().unwrap()[0].line,6);assert_eq!(m.types.as_ref().unwrap().len(),1);assert_eq!(m.kept.as_ref().unwrap().len(),1);assert!(m.final_);assert!(parse_mem(&json!({"t":"tick"})).is_none());}
#[test]
fn run_counts_growth_retention_and_reset(){let mut log=MemoryLog::default();log.begin(7);for(t,item,node)in [(1,10,5),(3,40,6),(5,90,5),(7,160,5)]{log.add(parse_mem(&json!({"t":"mem","peak":9000,"samples":[[t,1000]],"sites":[["a.py",2,7000,9]],"types":[["a.Item",item],["a.Node",node]]})).unwrap());}assert!(log.growing("a.Item"));assert!(!log.growing("a.Node"));assert_eq!(log.count("a.Item"),160.);assert_eq!(log.peak,9000.);assert_eq!(log.sites[0].line,2);assert!(log.running);log.add(parse_mem(&json!({"t":"mem","peak":1,"samples":[],"final":true})).unwrap());assert!(!log.running);log.begin(8);assert!(log.samples.is_empty());}
#[test]
fn readable_sizes(){assert_eq!(format_bytes(512.),"512 B");assert_eq!(format_bytes(2048.),"2.0 KB");assert_eq!(format_bytes(5.*1024.*1024.),"5.0 MB");assert_eq!(format_bytes(150.*1024.*1024.),"150 MB");}
