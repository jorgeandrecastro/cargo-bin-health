use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Default)]
pub struct MemoryRegion {
    pub name: String,
    pub origin: Option<u64>,
    pub length: Option<u64>,
    pub end: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryRegions {
    pub flash_bytes: Option<u64>,
    pub ram_bytes: Option<u64>,
    pub source: Option<String>,
    pub regions: Vec<MemoryRegion>,
}

fn clean(s: &str) -> String { s.split("/*").next().unwrap_or(s).split("//").next().unwrap_or("").trim().to_string() }
fn eval_num(s: &str) -> Option<u64> {
    let s = clean(s).trim().trim_end_matches(',').trim_end_matches(';').replace(' ', "");
    let s = s.trim_start_matches('(').trim_end_matches(')');
    if let Some((a,b)) = s.split_once('+') { return eval_num(a)?.checked_add(eval_num(b)?); }
    if let Some((a,b)) = s.split_once('-') { return eval_num(a)?.checked_sub(eval_num(b)?); }
    let (n,m) = if s.ends_with('K') || s.ends_with('k') { (&s[..s.len()-1],1024u64) }
        else if s.ends_with('M') || s.ends_with('m') { (&s[..s.len()-1],1024u64*1024) }
        else { (s,1) };
    let v = if let Some(x)=n.strip_prefix("0x").or_else(||n.strip_prefix("0X")) { u64::from_str_radix(x,16).ok()? } else { n.parse().ok()? };
    v.checked_mul(m)
}

pub fn parse_memory_x(input: &str) -> MemoryRegions {
    let mut result = MemoryRegions::default();
    let mut in_memory = false;
    let mut saw_memory = false;
    for raw in input.lines() {
        let line = clean(raw);
        if line.contains("MEMORY") { saw_memory = true; }
        if saw_memory && line.contains('{') { in_memory = true; }
        if saw_memory && line.trim() == "{" { in_memory = true; continue; }
        let line = if in_memory && line.contains("MEMORY") {
            line.split_once('{')
                .map(|x| x.1.trim().to_string())
                .unwrap_or_default()
        } else {
            line
        };
        if !in_memory { continue; }
        let end_of_memory = line.contains('}');
        let line = line.split('}').next().unwrap_or(&line).trim();
        if line.is_empty() {
            if end_of_memory {
                break;
            }
            continue;
        }
        let Some((name, body)) = line.split_once(':') else { continue };
        let upper = body.to_ascii_uppercase();
        let field = |key: &str| -> Option<u64> {
            let pos = upper.find(key)? + key.len();
            let tail = body[pos..].trim_start().trim_start_matches('=').trim_start();
            let token = tail.split([',',';']).next()?.trim();
            eval_num(token)
        };
        let origin = field("ORIGIN");
        let length = field("LENGTH");
        let end = origin.zip(length).and_then(|(o,l)|o.checked_add(l));
        let n = name.trim().to_string();
        let u = n.to_ascii_uppercase();
        if u.contains("FLASH") || u == "IROM" || u == "DROM" { result.flash_bytes = Some(result.flash_bytes.unwrap_or(0).max(length.unwrap_or(0))); }
        if u.contains("RAM") || u == "IRAM" || u == "DRAM" { result.ram_bytes = Some(result.ram_bytes.unwrap_or(0).max(length.unwrap_or(0))); }
        result.regions.push(MemoryRegion { name:n, origin, length, end });
        if end_of_memory {
            break;
        }
    }
    result
}

pub fn find_and_parse(workspace: &Path, package: &Path) -> Option<MemoryRegions> {
    let mut candidates = vec![package.join("memory.x"), workspace.join("memory.x"), package.join("build/memory.x")];
    // esp-hal generates the effective linker scripts under target/<triple>/build/<crate>/out.
    let target_dir = workspace.join("target");
    if let Ok(triples) = fs::read_dir(&target_dir) {
        for triple in triples.flatten() {
            let build = triple.path().join("build");
            if let Ok(crates) = fs::read_dir(build) {
                for krate in crates.flatten() {
                    let out = krate.path().join("out");
                    let p = out.join("memory.x");
                    if p.is_file() { candidates.push(p); }
                }
            }
        }
    }
    candidates.dedup();
    for path in candidates {
        if path.is_file() && let Ok(content) = fs::read_to_string(&path) {
            let mut parsed = parse_memory_x(&content);
            parsed.source = Some(path.display().to_string());
            if !parsed.regions.is_empty() { return Some(parsed); }
        }
    }
    None
}

#[cfg(test)]
mod tests {
 use super::*;
 #[test] fn parses_simple_regions() { let x=parse_memory_x("MEMORY {\n FLASH : ORIGIN = 0x08000000, LENGTH = 512K\n RAM : ORIGIN = 0x20000000, LENGTH = 256K\n}"); assert_eq!(x.flash_bytes,Some(512*1024)); assert_eq!(x.ram_bytes,Some(256*1024)); }
 #[test] fn parses_esp32c3_linker_regions_and_expressions() { let x=parse_memory_x("MEMORY\n{\n ICACHE : ORIGIN = 0x4037C000, LENGTH = 0x4000\n IRAM : ORIGIN = 0x4037C000 + 0x4000, LENGTH = 313K - 0x4000\n DRAM : ORIGIN = 0x3FC80000, LENGTH = 313K\n IROM : ORIGIN = 0x42000000 + 0x20, LENGTH = 0x400000 - 0x20\n DROM : ORIGIN = 0x3C000000 + 0x20, LENGTH = 0x400000 - 0x20\n}"); assert_eq!(x.regions.len(),5); assert_eq!(x.regions[1].origin,Some(0x40380000)); assert_eq!(x.regions[1].length,Some(313*1024-0x4000)); assert_eq!(x.regions[3].length,Some(0x400000-0x20)); }
}
