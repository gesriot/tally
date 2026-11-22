//! Ссылки команд, отмены и повтора ведут только на живые записи: корзина,
//! закрытые проекты, архивные разделы, предельные метки, старый формат журнала.

mod ambiguous_heading_names;
mod archived_heading;
mod closed_project_heading;
mod command_outcomes;
mod current_container_by_name;
mod heading_chain;
mod legacy_chain;
mod legacy_history_rows;
mod max_stamp_and_trashed_links;
mod namesake_headings;
mod no_change_and_validation;
mod service_validation;
