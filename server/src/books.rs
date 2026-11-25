//! Books: the library listing (core `Books` joined with the server's download, queue,
//! skip and backoff state — filtered, sorted and paged in SQL), and the metadata a
//! download needs.

use std::collections::HashMap;

use rust_core::pipeline::BookMeta;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::queue::SLOT_STATES_SQL;
use crate::template::TemplateValues;
use crate::time;

pub const STATUSES: &[&str] = &["downloaded", "queued", "downloading", "wanted", "cooldown", "failed", "skipped"];

/// `BookView` in API.md.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookView {
    pub id: String,
    pub provider: String,
    pub accounts: Vec<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub series: Option<String>,
    pub series_sequence: Option<String>,
    pub cover_url: Option<String>,
    pub length_minutes: i64,
    pub release_date: Option<String>,
    pub purchase_date: Option<String>,
    pub added_at: String,
    pub status: String,
    pub output_path: Option<String>,
    pub format: Option<String>,
    pub downloaded_at: Option<String>,
    pub last_error: Option<String>,
    pub backoff_until: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Title,
    Author,
    Added,
    Purchased,
    Released,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "title" => Sort::Title,
            "author" => Sort::Author,
            "added" => Sort::Added,
            "purchased" => Sort::Purchased,
            "released" => Sort::Released,
            _ => return None,
        })
    }
    fn column(self) -> &'static str {
        match self {
            Sort::Title => "title COLLATE NOCASE",
            Sort::Author => "first_author COLLATE NOCASE",
            Sort::Added => "created_at",
            Sort::Purchased => "purchase_date",
            Sort::Released => "date_published",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BookQuery {
    pub search: Option<String>,
    pub provider: Option<String>,
    pub account: Option<String>,
    pub statuses: Vec<String>,
    pub sort: Sort,
    pub desc: bool,
    pub page: i64,
    pub page_size: i64,
    /// Restrict to one book (detail view).
    pub id: Option<String>,
}

impl Default for BookQuery {
    fn default() -> Self {
        Self {
            search: None,
            provider: None,
            account: None,
            statuses: Vec::new(),
            sort: Sort::Added,
            desc: true,
            page: 1,
            page_size: 50,
            id: None,
        }
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    bid: i64,
    id: String,
    provider: String,
    title: String,
    subtitle: Option<String>,
    length_in_minutes: i64,
    date_published: Option<String>,
    created_at: String,
    cover: Option<String>,
    purchase_date: Option<String>,
    output_path: Option<String>,
    format: Option<String>,
    downloaded_at: Option<String>,
    qerror: Option<String>,
    backoff_until: Option<String>,
    status: String,
}

/// Escape `%`, `_` and `\` for a `LIKE ... ESCAPE '\'` pattern.
fn like_pattern(s: &str) -> String {
    let mut out = String::from("%");
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Bind values for the dynamic query, in order.
enum Bind {
    Text(String),
    Int(i64),
}

fn base_sql(q: &BookQuery, now: &str, binds: &mut Vec<Bind>) -> String {
    let mut sql = format!(
        "SELECT b.book_id AS bid, b.audible_product_id AS id, COALESCE(b.source, 'audible') AS provider,
                b.title, b.subtitle, b.length_in_minutes, b.date_published, b.created_at,
                COALESCE(b.picture_large, b.picture_id) AS cover,
                (SELECT MIN(ba.date_added) FROM core.BookAccounts ba WHERE ba.book_id = b.book_id AND ba.is_deleted = 0) AS purchase_date,
                d.output_path, d.format, d.downloaded_at,
                q.error AS qerror,
                CASE WHEN tb.until > ?1 THEN tb.until END AS backoff_until,
                CASE WHEN d.book_id IS NOT NULL THEN 'downloaded'
                     WHEN q.state IN ('planning','running') THEN 'downloading'
                     WHEN q.state IN ('queued','waiting') THEN 'queued'
                     WHEN s.book_id IS NOT NULL THEN 'skipped'
                     WHEN tb.until > ?1 THEN 'cooldown'
                     WHEN q.state = 'failed' THEN 'failed'
                     ELSE 'wanted' END AS status{}
         FROM core.Books b
         LEFT JOIN downloads d ON d.book_id = b.audible_product_id
         LEFT JOIN queue q ON q.book_id = b.audible_product_id AND q.state IN {SLOT_STATES_SQL}
         LEFT JOIN skips s ON s.book_id = b.audible_product_id
         LEFT JOIN title_backoff tb ON tb.book_id = b.audible_product_id
         WHERE b.content_type != 4
           AND (d.book_id IS NOT NULL OR EXISTS (
                SELECT 1 FROM core.BookAccounts ba JOIN core.Accounts a ON a.account_id = ba.account
                WHERE ba.book_id = b.book_id AND ba.is_deleted = 0))",
        if q.sort == Sort::Author {
            ",\n (SELECT c.name FROM core.BookContributors bc JOIN core.Contributors c ON c.contributor_id = bc.contributor_id
                  WHERE bc.book_id = b.book_id AND bc.role = 1 ORDER BY bc.\"order\" LIMIT 1) AS first_author"
        } else {
            ""
        }
    );
    binds.push(Bind::Text(now.to_string()));

    if let Some(id) = &q.id {
        binds.push(Bind::Text(id.clone()));
        sql.push_str(&format!(" AND b.audible_product_id = ?{}", binds.len()));
    }
    if let Some(p) = &q.provider {
        binds.push(Bind::Text(p.clone()));
        sql.push_str(&format!(" AND COALESCE(b.source, 'audible') = ?{}", binds.len()));
    }
    if let Some(a) = &q.account {
        binds.push(Bind::Text(a.clone()));
        sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM core.BookAccounts ba WHERE ba.book_id = b.book_id AND ba.is_deleted = 0 AND ba.account = ?{})",
            binds.len()
        ));
    }
    if let Some(term) = q.search.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        binds.push(Bind::Text(like_pattern(term)));
        let n = binds.len();
        sql.push_str(&format!(
            " AND (b.title LIKE ?{n} ESCAPE '\\' OR b.subtitle LIKE ?{n} ESCAPE '\\' OR b.audible_product_id LIKE ?{n} ESCAPE '\\'
                OR EXISTS (SELECT 1 FROM core.BookContributors bc JOIN core.Contributors c ON c.contributor_id = bc.contributor_id
                           WHERE bc.book_id = b.book_id AND bc.role IN (1, 2) AND c.name LIKE ?{n} ESCAPE '\\')
                OR EXISTS (SELECT 1 FROM core.SeriesBooks sb JOIN core.Series se ON se.series_id = sb.series_id
                           WHERE sb.book_id = b.book_id AND se.name LIKE ?{n} ESCAPE '\\'))"
        ));
    }
    sql
}

fn status_filter(q: &BookQuery, binds: &mut Vec<Bind>) -> String {
    if q.statuses.is_empty() {
        return String::new();
    }
    binds.push(Bind::Text(serde_json::to_string(&q.statuses).unwrap_or_else(|_| "[]".into())));
    format!(" WHERE status IN (SELECT value FROM json_each(?{}))", binds.len())
}

fn bind_all<'q, O>(
    mut query: sqlx::query::QueryAs<'q, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'q>>,
    binds: &'q [Bind],
) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'q>> {
    for b in binds {
        query = match b {
            Bind::Text(s) => query.bind(s.as_str()),
            Bind::Int(i) => query.bind(*i),
        };
    }
    query
}

/// One page of books plus the total matching count.
pub async fn list(pool: &SqlitePool, q: &BookQuery) -> sqlx::Result<(i64, Vec<BookView>)> {
    let now = time::now_str();
    let mut binds = Vec::new();
    let base = base_sql(q, &now, &mut binds);
    let filter = status_filter(q, &mut binds);

    let count_sql = format!("SELECT COUNT(*) FROM ({base}){filter}");
    let (total,): (i64,) = bind_all(sqlx::query_as(&count_sql), &binds).fetch_one(pool).await?;

    let dir = if q.desc { "DESC" } else { "ASC" };
    let mut page_binds = binds;
    page_binds.push(Bind::Int(q.page_size));
    let limit_n = page_binds.len();
    page_binds.push(Bind::Int((q.page - 1).max(0) * q.page_size));
    let offset_n = page_binds.len();
    // NULLs (no purchase/release date) sort last in either direction.
    let sql = format!(
        "SELECT * FROM ({base}){filter} ORDER BY ({col}) IS NULL, {col} {dir}, bid {dir} LIMIT ?{limit_n} OFFSET ?{offset_n}",
        col = q.sort.column()
    );
    let rows: Vec<Row> = bind_all(sqlx::query_as(&sql), &page_binds).fetch_all(pool).await?;
    let views = hydrate(pool, rows).await?;
    Ok((total, views))
}

pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<BookView>> {
    let q = BookQuery { id: Some(id.to_string()), page_size: 1, ..Default::default() };
    Ok(list(pool, &q).await?.1.into_iter().next())
}

/// The book's description as plain text.
pub async fn description(pool: &SqlitePool, id: &str) -> sqlx::Result<String> {
    let raw = sqlx::query_scalar::<_, String>("SELECT description FROM core.Books WHERE audible_product_id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .unwrap_or_default();
    Ok(html_to_text(&raw))
}

#[derive(Default)]
struct Relations {
    authors: Vec<String>,
    narrators: Vec<String>,
    publisher: Option<String>,
    series: Option<(String, Option<String>)>,
    accounts: Vec<String>,
}

/// Authors, narrators, series and owning accounts for a set of books, in four queries.
async fn relations(pool: &SqlitePool, ids: &[i64]) -> sqlx::Result<HashMap<i64, Relations>> {
    let mut map: HashMap<i64, Relations> = ids.iter().map(|id| (*id, Relations::default())).collect();
    if ids.is_empty() {
        return Ok(map);
    }
    let ids_json = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());

    let people: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT bc.book_id, bc.role, c.name FROM core.BookContributors bc
         JOIN core.Contributors c ON c.contributor_id = bc.contributor_id
         WHERE bc.book_id IN (SELECT value FROM json_each(?)) ORDER BY bc.book_id, bc.role, bc.\"order\"",
    )
    .bind(&ids_json)
    .fetch_all(pool)
    .await?;
    for (bid, role, name) in people {
        if let Some(r) = map.get_mut(&bid) {
            match role {
                1 => r.authors.push(name),
                2 => r.narrators.push(name),
                3 if r.publisher.is_none() => r.publisher = Some(name),
                _ => {}
            }
        }
    }

    let series: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT sb.book_id, s.name, sb.\"order\" FROM core.SeriesBooks sb
         JOIN core.Series s ON s.series_id = sb.series_id
         WHERE sb.book_id IN (SELECT value FROM json_each(?)) ORDER BY sb.book_id, s.series_id",
    )
    .bind(&ids_json)
    .fetch_all(pool)
    .await?;
    for (bid, name, order) in series {
        if let (Some(r), Some(name)) = (map.get_mut(&bid), name) {
            if r.series.is_none() && !name.trim().is_empty() {
                r.series = Some((name, order.filter(|o| !o.trim().is_empty())));
            }
        }
    }

    let owners: Vec<(i64, String)> = sqlx::query_as(
        "SELECT ba.book_id, ba.account FROM core.BookAccounts ba
         JOIN core.Accounts a ON a.account_id = ba.account
         WHERE ba.is_deleted = 0 AND ba.book_id IN (SELECT value FROM json_each(?)) ORDER BY ba.book_id, ba.account",
    )
    .bind(&ids_json)
    .fetch_all(pool)
    .await?;
    for (bid, account) in owners {
        if let Some(r) = map.get_mut(&bid) {
            r.accounts.push(account);
        }
    }
    Ok(map)
}

async fn hydrate(pool: &SqlitePool, rows: Vec<Row>) -> sqlx::Result<Vec<BookView>> {
    let ids: Vec<i64> = rows.iter().map(|r| r.bid).collect();
    let mut rel = relations(pool, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let rels = rel.remove(&r.bid).unwrap_or_default();
            let (series, seq) = match rels.series {
                Some((s, o)) => (Some(s), o),
                None => (None, None),
            };
            let show_error = matches!(r.status.as_str(), "failed" | "queued" | "downloading");
            BookView {
                id: r.id,
                provider: r.provider,
                accounts: rels.accounts,
                title: r.title,
                subtitle: r.subtitle.filter(|s| !s.trim().is_empty()),
                authors: rels.authors,
                narrators: rels.narrators,
                series,
                series_sequence: seq,
                cover_url: r.cover.filter(|c| !c.trim().is_empty()),
                length_minutes: r.length_in_minutes,
                release_date: r.date_published.filter(|d| !d.is_empty()),
                purchase_date: r.purchase_date.as_deref().map(time::api_from_core),
                added_at: time::api_from_core(&r.created_at),
                status: r.status,
                output_path: r.output_path,
                format: r.format,
                downloaded_at: r.downloaded_at,
                last_error: if show_error { r.qerror } else { None },
                backoff_until: r.backoff_until,
            }
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct MetaRow {
    book_id: i64,
    title: String,
    subtitle: Option<String>,
    description: String,
    date_published: Option<String>,
    language: Option<String>,
    cover: Option<String>,
    source: String,
}

/// Download metadata for one book, and its provider.
pub async fn meta(pool: &SqlitePool, product_id: &str) -> sqlx::Result<Option<(BookMeta, String)>> {
    let row: Option<MetaRow> = sqlx::query_as(
        "SELECT book_id, title, subtitle, description, date_published, language,
                COALESCE(picture_large, picture_id) AS cover, COALESCE(source, 'audible') AS source
         FROM core.Books WHERE audible_product_id = ?",
    )
    .bind(product_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let mut rel = relations(pool, &[row.book_id]).await?;
    let r = rel.remove(&row.book_id).unwrap_or_default();
    let (series, series_sequence) = match r.series {
        Some((s, o)) => (Some(s), o),
        None => (None, None),
    };
    let meta = BookMeta {
        product_id: product_id.to_string(),
        title: if row.title.trim().is_empty() { product_id.to_string() } else { row.title },
        subtitle: row.subtitle.filter(|s| !s.trim().is_empty()),
        authors: r.authors,
        narrators: r.narrators,
        series,
        series_sequence,
        publisher: r.publisher,
        release_date: row.date_published.filter(|d| !d.is_empty()),
        description: Some(html_to_text(&row.description)).filter(|d| !d.is_empty()),
        language: row.language,
        genres: Vec::new(),
        asin: (row.source == "audible").then(|| product_id.to_string()),
        isbn: (row.source == "librofm").then(|| product_id.to_string()),
        cover_url: row.cover.filter(|c| !c.trim().is_empty()),
    };
    Ok(Some((meta, row.source)))
}

/// Store descriptions can be HTML (Audible's are). Reduce one to plain text: block
/// ends become line breaks, other tags are dropped, common entities decoded.
pub fn html_to_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '<' {
            let next = s[i + 1..].chars().next();
            if next.is_some_and(|n| n.is_ascii_alphabetic() || n == '/' || n == '!') {
                let end = s[i..].find('>').map(|e| i + e);
                let Some(end) = end else { break };
                let tag = s[i + 1..end].trim_start_matches('/').to_ascii_lowercase();
                let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if matches!(name.as_str(), "br" | "p" | "div" | "li" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4") {
                    out.push('\n');
                }
                while chars.peek().is_some_and(|(j, _)| *j <= end) {
                    chars.next();
                }
                continue;
            }
        }
        if c == '&' {
            if let Some(semi) = s[i..].find(';').filter(|n| *n <= 10) {
                let entity = &s[i + 1..i + semi];
                let decoded = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    e if e.starts_with("#x") || e.starts_with("#X") => {
                        u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)
                    }
                    e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
                    _ => None,
                };
                if let Some(d) = decoded {
                    out.push(d);
                    while chars.peek().is_some_and(|(j, _)| *j <= i + semi) {
                        chars.next();
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }
    // Tidy whitespace: trim lines, at most one blank line in a row.
    let mut text = String::new();
    let mut blank = 0;
    for line in out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")) {
        if line.is_empty() {
            blank += 1;
            if blank > 1 || text.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        text.push_str(&line);
        text.push('\n');
    }
    text.trim_end().to_string()
}

/// Folder-template values for a book.
pub fn template_values(meta: &BookMeta, provider: &str) -> TemplateValues {
    TemplateValues {
        authors: meta.authors.clone(),
        narrators: meta.narrators.clone(),
        title: meta.title.clone(),
        subtitle: meta.subtitle.clone().unwrap_or_default(),
        series: meta.series.clone().unwrap_or_default(),
        series_sequence: meta.series_sequence.clone().unwrap_or_default(),
        year: meta
            .release_date
            .as_deref()
            .and_then(|d| d.get(..4))
            .filter(|y| y.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or_default()
            .to_string(),
        asin: meta.asin.clone().unwrap_or_default(),
        provider: provider.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_descriptions_become_plain_text() {
        let html = "<p>A <b>bold</b> start &amp; more.</p><p>Second&nbsp;para &#8212; <i>end</i></p><br/>";
        assert_eq!(html_to_text(html), "A bold start & more.\n\nSecond para \u{2014} end");
        assert_eq!(html_to_text("Plain 3 < 4 text"), "Plain 3 < 4 text");
        assert_eq!(html_to_text("Fish &chips; ok"), "Fish &chips; ok");
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("50%_off\\"), "%50\\%\\_off\\\\%");
    }

    #[test]
    fn template_values_take_the_year_from_the_release_date() {
        let meta = BookMeta {
            product_id: "B0".into(),
            title: "T".into(),
            release_date: Some("2006-07-17".into()),
            ..Default::default()
        };
        assert_eq!(template_values(&meta, "audible").year, "2006");
        let meta = BookMeta { release_date: Some("n/a".into()), ..meta };
        assert_eq!(template_values(&meta, "audible").year, "");
    }
}
