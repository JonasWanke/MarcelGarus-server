use crate::utils::*;
use chrono::{Date, Datelike, Utc};
use comrak::{
    arena_tree::Children,
    nodes::{Ast, AstNode, ListType, NodeValue},
    parse_document, Arena, ComrakOptions,
};
use log::{info, warn};
use std::{cell::RefCell, collections::HashMap, sync::RwLock};

#[derive(Clone, Debug)]
pub struct Article {
    pub key: String,
    pub title: String,
    pub published: Date<Utc>,
    pub content: String,
}

/// A database for articles. It gets articles from the GitHub repo.
pub struct ArticleDb {
    articles: HashMap<String, Article>,
}
impl ArticleDb {
    const BASE_URL: &'static str =
        "https://raw.githubusercontent.com/marcelgarus/server/master/blog";

    pub async fn new() -> Self {
        let mut db = Self {
            articles: Default::default(),
        };
        db.load().await.unwrap();
        db
    }

    async fn load(&mut self) -> Result<(), String> {
        let keys = download(&format!("{}/published.md", Self::BASE_URL))
            .await?
            .split('\n')
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| match article_line::to_key_and_date(line) {
                Ok(it) => Some(it),
                Err(err) => {
                    warn!("Invalid article line {}: {:?}", line, err);
                    None
                }
            })
            .collect::<Vec<(String, Date<Utc>)>>();

        let mut articles: HashMap<String, Article> = Default::default();
        for (key, date) in keys {
            let git_url = format!(
                "{}/{:04}-{:02}-{:02}-{}.md",
                Self::BASE_URL,
                date.year(),
                date.month(),
                date.day(),
                key
            );
            info!("Fetching article from {}", git_url);
            let content = download(&git_url).await?;
            articles.insert(
                key.clone(),
                Article::from_key_and_date_and_markdown(key, date, &content),
            );
        }

        info!(
            "Loaded articles: {}",
            itertools::join(articles.keys(), ", ")
        );
        self.articles = articles;
        Ok(())
    }

    pub fn article_for(&self, key: &str) -> Option<Article> {
        self.articles.get(key).map(|article| article.clone())
    }

    pub fn list(&self) -> Vec<Article> {
        self.articles
            .values()
            .map(|article| article.clone())
            .collect()
    }
}

impl Article {
    fn from_key_and_date_and_markdown(key: String, date: Date<Utc>, markdown: &str) -> Self {
        let arena = Arena::new();
        let root = parse_document(&arena, markdown, &ComrakOptions::default());

        let content = {
            let mut html = vec![];
            root.to_html(&mut html);
            itertools::join(html, "")
        };
        Self {
            key,
            title: "The Title".into(),
            published: date,
            content,
        }
    }
}
trait ToHtml<'a> {
    fn to_html(&'a self, output: &mut Vec<String>);
}
impl<'a> ToHtml<'a> for AstNode<'a> {
    fn to_html(&'a self, output: &mut Vec<String>) {
        use NodeValue::*;
        match self.data.borrow().value.clone() {
            Document => self.children().to_html(output),
            Heading(heading) => {
                if heading.level == 1 {
                    return; // The title of the entire article is treated separately.
                }
                heading.level;
                output.start_tag("h2");
                self.children().to_html(output);
                output.end_tag("h2");
            }
            Paragraph => {
                output.start_tag("p");
                self.children().to_html(output);
                output.end_tag("p");
            }
            // TODO: Html-encode content
            Text(text) => output.push(text.utf8_or_panic()),
            SoftBreak => output.push(" ".into()),
            LineBreak => output.push("<br />".into()),
            Emph => {
                output.start_tag("em");
                self.children().to_html(output);
                output.end_tag("em");
            }
            Strong => {
                output.start_tag("strong");
                self.children().to_html(output);
                output.end_tag("strong");
            }
            List(list) => {
                let tag = match list.list_type {
                    ListType::Bullet => "ul",
                    ListType::Ordered => "ol",
                }
                .to_owned();
                output.start_tag(&tag);
                self.children().to_html(output);
                output.end_tag(&tag);
            }
            Item(_) => {
                output.start_tag("li");
                self.children().to_html(output);
                output.end_tag("li");
            }
            HtmlBlock(it) => output.push(format!("{}", it.literal.utf8_or_panic())),
            ThematicBreak => output.push("<hr />".into()),
            Link(link) => {
                output.start_tag(&format!("a href=\"{}\"", link.url.utf8_or_panic(),));
                output.push(link.title.utf8_or_panic());
                self.children().to_html(output);
                output.end_tag("a");
            }
            Image(image) => {
                output.push(format!(
                    "<img src=\"{}\" alt=\"{}\" />",
                    image.url.utf8_or_panic(),
                    image.title.utf8_or_panic()
                ));
            }
            Code(code) => {
                output.start_tag("code");
                // TODO: Html-encode content
                output.push(format!("{}", code.utf8_or_panic()));
                output.end_tag("code");
            }
            CodeBlock(code) => {
                output.start_tag("pre");
                output.start_tag(&format!(
                    "code class=\"language-{}\"",
                    code.info.utf8_or_panic()
                ));
                // TODO: Html-encode content
                output.push(format!("{}", code.literal.utf8_or_panic()));
                output.end_tag("code");
                output.end_tag("pre");
            }
            _ => {
                warn!("Not handling node {:?} yet.", self);
            }
        }
    }
}
trait ChildrenToHtml<'a> {
    fn to_html(self, output: &mut Vec<String>);
}
impl<'a> ChildrenToHtml<'a> for Children<'a, RefCell<Ast>> {
    fn to_html(self, output: &mut Vec<String>) {
        for child in self.collect::<Vec<_>>().clone() {
            child.to_html(output)
        }
    }
}
trait TagVecExt {
    fn start_tag(&mut self, tag: &str);
    fn end_tag(&mut self, tag: &str);
}
impl TagVecExt for Vec<String> {
    fn start_tag(&mut self, tag: &str) {
        self.push(format!("<{}>", tag));
    }
    fn end_tag(&mut self, tag: &str) {
        self.push(format!("</{}>", tag));
    }
}

mod article_line {
    use chrono::{Date, TimeZone, Utc};
    use nom::{
        bytes::complete::tag, character::complete::digit1, combinator::map_res, sequence::tuple,
        IResult,
    };

    pub fn to_key_and_date(line: &str) -> Result<(String, Date<Utc>), String> {
        parse(line)
            .map(|it| it.1)
            .map_err(|err| format!("Error while parsing article id: {:?}", err))
    }
    fn parse(input: &str) -> IResult<&str, (String, Date<Utc>)> {
        let (input, (year, _, month, _, day, _)) = tuple((
            map_res(digit1, |it: &str| it.parse::<i32>()),
            tag("-"),
            map_res(digit1, |it: &str| it.parse::<u32>()),
            tag("-"),
            map_res(digit1, |it: &str| it.parse::<u32>()),
            tag("-"),
        ))(input)?;
        Ok(("", (input.to_owned(), Utc.ymd(year, month, day))))
    }
}

/// Articles look like this: GET /article-key
/// You can get an overview of all articles like this: GET/blog
pub struct Handler {
    db: RwLock<ArticleDb>,
}
impl Handler {
    pub async fn new() -> Self {
        Self {
            db: RwLock::new(ArticleDb::new().await),
        }
    }
    pub async fn handle(&self, request: &Request) -> Option<Response> {
        if request.method == Method::GET {
            if request.path.len() != 1 {
                return None;
            }
            let key: String = request.path.get(0).unwrap().into();
            let article = self.db.read().unwrap().article_for(&key)?;
            info!("Reading article {}", article.key);
            let template: String =
                String::from_utf8(std::fs::read("assets/article.html").unwrap()).unwrap();
            let article = template
                .replace("{{title}}", &article.title)
                .replace(
                    "{{publish-date}}",
                    &format!("{}", article.published.format("%Y-%m-%d")),
                )
                .replace("{{body}}", &article.content);
            return Some(hyper::Response::with_body(article.into()));
        }

        None
    }
}
