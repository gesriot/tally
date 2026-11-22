//! Общие помощники интеграционных тестов.

#![allow(dead_code)]

use tally_core::{
    Area, AreaId, Heading, HeadingId, Project, ProjectId, Rank, Tag, TagId, Task, TaskId, Timestamp,
};

pub fn at(s: &str) -> Timestamp {
    s.parse().unwrap_or(Timestamp::UNIX_EPOCH)
}

pub fn t0() -> Timestamp {
    at("2026-11-19T09:00:00Z")
}

pub fn rank(s: &str) -> Rank {
    s.parse().unwrap_or_else(|_| Rank::initial())
}

pub fn task(n: u128, title: &str) -> Task {
    Task::new(TaskId::from_u128(n), title, Rank::initial(), t0())
}

pub fn project(n: u128, title: &str) -> Project {
    Project::new(ProjectId::from_u128(n), title, Rank::initial(), t0())
}

pub fn area(n: u128, title: &str) -> Area {
    Area::new(AreaId::from_u128(n), title, Rank::initial(), t0())
}

pub fn tag(n: u128, title: &str) -> Tag {
    Tag::new(TagId::from_u128(n), title, Rank::initial(), t0())
}

pub fn heading(n: u128, project: ProjectId, title: &str) -> Heading {
    Heading::new(
        HeadingId::from_u128(n),
        project,
        title,
        Rank::initial(),
        t0(),
    )
}
