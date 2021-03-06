#![allow(unreachable_patterns)]
use super::convert;
use super::{ArchivedRequest, RequestFacts};
use failure::Error;
use har::v1_2::*;
use har::{Har, Spec};
use hyper::{Method, Uri};
use serde_json;
use slog::Logger;
use std::fs;
use std::fs::File;
use std::io::{Error as IoError, ErrorKind, Result as IoResult};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Fail)]
pub enum HarLoadingError {
    #[fail(display = "Invalid HAR version")]
    InvalidVersion,
    #[fail(display = "Couldn't create matcher: {}", _0)]
    InvalidMatcher(String),
}

pub struct HarLoader {
    logger: Logger,
}
impl HarLoader {
    pub fn new(logger: Logger) -> HarLoader {
        HarLoader { logger }
    }

    fn find_requests<P: AsRef<Path>>(&self, path: P) -> IoResult<Vec<PathBuf>> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(IoError::new(ErrorKind::NotFound, "Path is not a directory"));
        }
        trace!(self.logger, "Reading files in {:?}", &path);
        let mut results = Vec::new();
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let path = entry.path();
            trace!(self.logger, "Examining {:?}", &path);
            if path.is_file() && path.to_string_lossy().ends_with(".json") {
                trace!(self.logger, "Accepted {:?}", &path);
                results.push(path);
            }
        }
        Ok(results)
    }

    pub fn load(&self, path: &Path) -> Result<Vec<ArchivedRequest>, Error> {
        trace!(self.logger, "Loading transaction from {:?}", &path);
        let f = File::open(&path)?;
        let har: Har = serde_json::from_reader(f)?;
        trace!(self.logger, "Loaded HAR for {:?}", &path);
        if let Spec::V1_2(log) = har.log {
            let fname = path.to_string_lossy().into_owned();
            info!(self.logger, "Found HAR v1.2 with {} entries", log.entries.len(); "path" => fname);
            log.entries.iter().map(|e| self.load_entry(e)).collect()
        } else {
            Err(HarLoadingError::InvalidVersion.into())
        }
    }

    fn get_facts(&self, r: &Request) -> Result<Vec<RequestFacts>, Error> {
        let mut results = Vec::new();
        let method = Method::from_bytes(&r.method.to_uppercase().as_bytes())
            .map_err(|_| HarLoadingError::InvalidMatcher(format!("Unknown method {}", r.method)))?;
        results.push(RequestFacts::Method(method));

        let uri = Uri::from_str(&r.url)
            .map_err(|_| HarLoadingError::InvalidMatcher(format!("Invalid Uri {}", r.url)))?;
        let path = uri
            .path_and_query()
            .map(|pq| format!("{}", pq))
            .unwrap_or_else(|| "".to_string());
        results.push(RequestFacts::PathAndQuery(path));

        if let Some(d) = &r.post_data {
            let (data, content_type) = convert::RequestBody::bytes(&d)?;
            results.push(RequestFacts::Body { data, content_type });
        }

        // TODO: figure out if we care about Headers

        Ok(results)
    }

    fn load_entry(&self, e: &Entries) -> Result<ArchivedRequest, Error> {
        let timing = if e.time < 0 {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(e.time as u64)
        };
        Ok(ArchivedRequest {
            original_timing: timing,
            facts: self.get_facts(&e.request)?,
            response: e.response.clone(),
        })
    }

    pub fn load_all<P: AsRef<Path>>(&self, path: P) -> Result<Vec<ArchivedRequest>, Error> {
        let path = path.as_ref();
        trace!(self.logger, "Loading all interactions from {:?}", &path);
        let files = self.find_requests(&path)?;
        let mut results = Vec::new();
        for f in files {
            results.extend(self.load(&f)?);
        }
        Ok(results)
    }
}
