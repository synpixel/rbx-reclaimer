use async_recursion::async_recursion;
use clap::Parser;
use colored::{Color, Colorize};
use rand::seq::SliceRandom;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::Duration;
use terminal_link::Link;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct User {
    has_verified_badge: bool,
    user_id: u32,
    username: String,
    display_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Shout {
    body: String,
    poster: User,
    created: String,
    updated: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Group {
    id: u32,
    name: String,
    description: String,
    owner: Option<User>,
    shout: Option<Shout>,
    member_count: u32,
    is_builders_club_only: bool,
    public_entry_allowed: bool,
    is_locked: Option<bool>,
    has_verified_badge: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Relationships {
    group_id: u32,
    relationship_type: String,
    total_group_count: u32,
    related_groups: Vec<Group>,
    next_row_index: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RobloxError {
    code: u32,
    message: String,
    user_facing_message: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GroupOwnershipResponseBody {
    errors: Option<Vec<RobloxError>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct GroupSearchResponseItem {
    id: u32,
    name: String,
    description: String,
    member_count: u32,
    previous_name: Option<String>,
    public_entry_allowed: bool,
    created: String,
    updated: String,
    has_verified_badge: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GroupSearchResponse {
    keyword: Option<String>,
    previous_page_cursor: Option<String>,
    next_page_cursor: Option<String>,
    data: Option<Vec<GroupSearchResponseItem>>,
    errors: Option<Vec<RobloxError>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ArrayGroupResponseItem {
    id: u32,
    name: String,
    description: String,
    owner: Option<User>,
    created: String,
    has_verified_badge: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ArrayGroupResponse {
    data: Vec<ArrayGroupResponseItem>,
    errors: Option<Vec<RobloxError>>,
}

/// Roblox unclaimed group finder
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The query to look groups with
    #[arg(short, long)]
    query: Option<String>,

    /// Minimum group id
    #[arg(long, default_value_t = 1)]
    min: u32,

    /// Maximum group id
    #[arg(long, default_value_t = 17064733)]
    max: u32,

    /// Whether or not to ignore closed groups
    #[arg(long)]
    ignore_closed_groups: bool,

    /// Which group api domain to send requests to
    #[arg(short, long, default_value_t = String::from("https://groups.roblox.com"))]
    group_api_domain: String,

    /// Whether or not to repeat the search infinitely
    #[arg(short, long)]
    repeat: bool,
}

#[async_recursion(?Send)]
async fn get_random_group_id(
    args: &Args,
    next_page_cursor: Option<String>,
    client: &Client,
) -> Result<u32, Box<dyn std::error::Error>> {
    if args.query.is_some() {
        let empty_string = String::new();

        let group_results = client
            .get(format!(
                "{}/v1/groups/search?keyword={}&prioritizeExactMatch=false&limit=100&cursor={}",
                args.group_api_domain,
                args.query.as_ref().unwrap(),
                if next_page_cursor.is_some() {
                    next_page_cursor.unwrap()
                } else {
                    empty_string
                }
            ))
            .send()
            .await?
            .json::<GroupSearchResponse>()
            .await;

        if let Ok(group_results) = group_results {
            if group_results.errors.is_some() {
                panic!("{:?}", group_results.errors);
            }

            let group_ids: Vec<u32> = group_results
                .data
                .unwrap()
                .iter()
                .map(|group| &group.id)
                .cloned()
                .collect();

            if let Ok(groups) = fetch_groups(group_ids, args, client).await {
                let data: Vec<Group> = groups
                    .iter()
                    .filter(|group| is_group_available(group, args))
                    .cloned()
                    .collect();

                if !data.is_empty() {
                    return Ok(data.choose(&mut rand::thread_rng()).unwrap().id);
                } else if group_results.next_page_cursor.is_some() {
                    return get_random_group_id(args, group_results.next_page_cursor, client).await;
                } else {
                    println!("{}", "No groups to look through".red());
                }
            }
        }
    } else {
        return Ok(rand::thread_rng().gen_range(args.min..=args.max));
    }

    Ok(0)
}

async fn fetch_groups(
    group_ids: Vec<u32>,
    args: &Args,
    client: &Client,
) -> Result<Vec<Group>, Box<dyn std::error::Error>> {
    let mut groups: Vec<Group> = vec![];

    for group_id in group_ids.iter() {
        let group = client
            .get(format!("{}/v1/groups/{}", args.group_api_domain, group_id))
            .send()
            .await?
            .json::<Group>()
            .await;

        if let Ok(group) = group {
            groups.push(group);
        }
    }

    Ok(groups)
}

fn is_group_available(group: &Group, args: &Args) -> bool {
    if group.owner.is_some() || group.is_locked.is_some() {
        return false;
    }

    if args.ignore_closed_groups && (!group.public_entry_allowed || group.member_count == 0) {
        return false;
    }

    true
}

fn exclude_group(group_id: u32) -> Result<(), Box<dyn std::error::Error>> {
    if !Path::new("groups.json").exists() {
        let mut file = File::create("groups.json")?;
        file.write_all("[]".as_bytes())?;
    }

    let contents = fs::read_to_string("groups.json")?;

    let mut group_ids: Vec<u32> = serde_json::from_str(contents.as_str())?;
    group_ids.push(group_id);

    let new_group_ids = serde_json::to_string(&group_ids)?;
    fs::write("groups.json", new_group_ids)?;

    Ok(())
}

fn is_group_excluded(group_id: u32) -> Result<bool, Box<dyn std::error::Error>> {
    if !Path::new("groups.json").exists() {
        let mut file = File::create("groups.json")?;
        file.write_all("[]".as_bytes())?;
    }

    let group_ids: Vec<u32> = serde_json::from_str(fs::read_to_string("groups.json")?.as_str())?;
    Ok(group_ids.contains(&group_id))
}

async fn process_group(
    group: &Group,
    args: &Args,
    client: &Client,
) -> Result<bool, Box<dyn std::error::Error>> {
    if is_group_excluded(group.id).unwrap_or_else(|err| {
        panic!(
            "Failed to check for group {} in groups.json: {}",
            group.id, err
        )
    }) {
        return Ok(false);
    }

    exclude_group(group.id)
        .unwrap_or_else(|err| panic!("Failed to exclude group {}: {}", group.id, err));

    process_relationships(group, args, client)
        .await
        .expect("Failed to process relationships.");

    if !is_group_available(group, args) {
        return Ok(false);
    }

    let separator = "│".truecolor(140, 140, 140);

    println!(
        "{} {separator} {:<8} {separator} {:<6} {separator} {}",
        Link::new(
            format!("{:<50}", group.name.blue()).as_str(),
            format!("https://www.roblox.com/groups/{}", group.id).as_str()
        ),
        group.id,
        if group.public_entry_allowed {
            "Open".green()
        } else {
            "Closed".red()
        },
        format!("{} Members", group.member_count).color(if group.member_count > 0 {
            Color::Green
        } else {
            Color::Red
        })
    );

    Ok(true)
}

#[async_recursion(?Send)]
async fn process_relationships(
    group: &Group,
    args: &Args,
    client: &Client,
) -> Result<(), Box<dyn std::error::Error>> {
    let allies = client
        .get(format!(
            "{}/v1/groups/{}/relationships/allies?StartRowIndex=1&MaxRows=100",
            args.group_api_domain, group.id
        ))
        .send()
        .await?
        .json::<Relationships>()
        .await;

    let enemies = client
        .get(format!(
            "{}/v1/groups/{}/relationships/enemies?StartRowIndex=1&MaxRows=100",
            args.group_api_domain, group.id
        ))
        .send()
        .await?
        .json::<Relationships>()
        .await;

    if let Ok(allies) = allies {
        for ally in allies.related_groups.iter() {
            process_group(ally, args, client).await?;
        }
    }

    if let Ok(enemies) = enemies {
        for enemy in enemies.related_groups.iter() {
            process_group(enemy, args, client).await?;
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let client = Client::new();
    let interval = Duration::from_secs_f64(0.);

    env_logger::init();

    loop {
        let group_id = get_random_group_id(&args, None, &client).await.unwrap();

        let group = client
            .get(format!("{}/v1/groups/{}", args.group_api_domain, group_id))
            .send()
            .await?
            .json::<Group>()
            .await;

        if let Ok(group) = group {
            if let Ok(success) = process_group(&group, &args, &client).await {
                if success && !args.repeat {
                    break;
                }
            }
        }

        thread::sleep(interval);
    }

    Ok(())
}
