use base64::{engine::general_purpose, Engine as _};
use clap::{arg, Command};
use comrak::{markdown_to_html, ComrakOptions};
use mail_builder::headers as b_headers;
use mail_builder::headers::HeaderType;
use mail_builder::MessageBuilder;
use mail_parser::{Addr, HeaderName, HeaderValue, Message, MessagePart, PartType, RfcHeader};
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

fn cli() -> Command {
    Command::new("cargo")
        .about("Email enrich tool for mutt")
        .args(vec![
            arg!(<FILE> "path to email file  (use '-' for stdin)"),
            arg!(--"get-message-id" "Prints message id of given mail"),
            arg!(--"get-subject" "Prints subject of given mail"),
            arg!(--"get-from-email" "Prints from email of given mail"),
            arg!(--"html-preview" "Generate html from markdown in text body and prints it"),
            arg!(--"generate-html" "Generate html body from markdown in text body"),
            arg!(--"add-pixel" <BASE_URL> "Add tracking pixel to html body")
                .requires("generate-html"),
            arg!(--"put-on-imap" <MAILBOX> "Put email on IMAP server")
                .requires("server")
                .requires("port")
                .requires("user")
                .requires("password"),
            arg!(--server <SERVER> "IMAP server uri"),
            arg!(--port <PORT> "IMAP server port"),
            arg!(--user <USER> "IMAP user name"),
            arg!(--password <PASS> "IMAP password"),
        ])
}

fn main() {
    let matches = cli().get_matches();

    let file = matches
        .get_one::<String>("FILE")
        .map_or_else(|| panic!("No email file provided"), get_email_content);

    let message = Message::parse(file.as_slice()).unwrap();

    if matches.get_flag("get-message-id") {
        println!("{}", message.message_id().unwrap_or(""));
        return;
    }

    if matches.get_flag("get-subject") {
        println!("{}", message.subject().unwrap_or(""));
        return;
    }

    if matches.get_flag("get-from-email") {
        match message.from() {
            HeaderValue::Address(from) => {
                let email = from
                    .address
                    .clone()
                    .map_or_else(String::new, std::borrow::Cow::into_owned);
                println!("{}", email);
            }
            _ => println!(),
        }
        return;
    }

    if matches.get_flag("html-preview") {
        println!("{}", text_body_as_html(&message, None));
        return;
    }

    let mut eml = get_builder_from_parser(&message);

    handle_put_email_on_imap_server(&eml, &message, &matches);

    let append = matches
        .get_one::<String>("add-pixel")
        .map(|tracking_url| get_pixel_element(tracking_url, &message));

    if matches.get_flag("generate-html") {
        eml = eml.html_body(text_body_as_html(&message, append));
    }

    println!("{}", eml.write_to_string().unwrap());
}

fn text_body(message: &Message) -> String {
    message.body_text(0).unwrap().to_string()
}

fn pre_markdown(text: &str) -> String {
    let mut result = String::new();
    text.lines().for_each(|line| {
        // append two spaces to force line break
        result.push_str(&format!("{line}  \n"));
    });
    result
}

fn text_body_as_html(message: &Message, append: Option<String>) -> String {
    let body = markdown_to_html(
        &pre_markdown(&text_body(message)),
        &ComrakOptions::default(),
    );
    let body_append = append.map_or_else(String::new, |append| append);
    format!(
        r#"
        <html>
            <head>
            <meta http-equiv="Content-Type" content="text/html charset=UTF-8" />
            <meta name="generator" content="mutt-html-markdown/0.1" />
            <style>
                code {{ margin-left: 20px; background: #ddd; display: inline-block; padding: 10px 16px; font-family: monospace; }}
                blockquote {{ white-space: normal; border-left: 10px solid #ddd; margin-left: 0; padding-left: 10px }}
            </style>
            </head>
            <body>
                {}
                {}
            </body>
        </html>
        "#,
        body, body_append
    )
}

fn transform_address<'a>(address: &'a Addr) -> b_headers::address::Address<'a> {
    let name = address.name.as_ref().map(AsRef::as_ref);
    b_headers::address::Address::new_address(name, address.address.as_ref().unwrap().clone())
}

fn copy_headers<'a>(mut dest: MessageBuilder<'a>, source: &'a Message) -> MessageBuilder<'a> {
    for header in source.headers() {
        let maybe_header = match header.value() {
            HeaderValue::Address(address) => Some(HeaderType::Address(transform_address(address))),
            HeaderValue::Text(text) => {
                Some(HeaderType::Text(b_headers::text::Text::new(text.as_ref())))
            }
            HeaderValue::DateTime(datetime) => Some(HeaderType::Date(b_headers::date::Date::new(
                datetime.to_timestamp(),
            ))),
            // content will be generated automatically, it will mess up email if copied here
            HeaderValue::ContentType(_) => None,
            HeaderValue::AddressList(addresses) => {
                let mut new_addresses = vec![];
                for address in addresses.iter() {
                    let new_address = transform_address(address);
                    new_addresses.push(new_address);
                }
                Some(HeaderType::Address(b_headers::address::Address::List(
                    new_addresses,
                )))
            }
            HeaderValue::Group(group) => todo!("Group not implemented {:?}", group),
            HeaderValue::GroupList(group_list) => {
                todo!("Group list not implemented {:?}", group_list)
            }
            HeaderValue::TextList(text_list) => {
                let text = text_list.join("\t\n");
                Some(HeaderType::Text(b_headers::text::Text::new(text)))
            }
            HeaderValue::Empty => todo!("Empty not implemented"),
        };
        if let Some(new_header) = maybe_header {
            dest = dest.header(header.name(), new_header);
        };
    }
    dest
}

fn get_email_content(file_path: &String) -> Vec<u8> {
    if file_path == "-" {
        let stdin = std::io::stdin();
        let mut input = String::new();

        while let Ok(n) = stdin.read_line(&mut input) {
            if n == 0 {
                break;
            }
        }
        input.as_bytes().to_vec()
    } else {
        let mut file_content = vec![];
        let path = Path::new(file_path);
        let mut fh = File::open(path).expect("Unable to open file");
        fh.read_to_end(&mut file_content).expect("Unable to read");
        file_content
    }
}

fn get_pixel_element(tracking_url: &String, message: &Message) -> String {
    let encoded_id: String = general_purpose::STANDARD_NO_PAD.encode(message.message_id().unwrap());
    let pixel_url = format!("{}/image/{}.gif", tracking_url, encoded_id);
    format!(
        r#"
        <img src="{}" alt="Open pixel" style="border: 0px; width: 0px; max-width: 1px;" />
        "#,
        pixel_url
    )
}

fn put_email_on_imap_server(
    eml: MessageBuilder,
    mailbox: &String,
    server: &String,
    port: u16,
    user: &String,
    pass: &String,
) {
    let tls = native_tls::TlsConnector::builder().build().unwrap();
    let client = imap::connect((server.clone(), port), server, &tls).unwrap();
    let mut imap_session = client.login(user, pass).map_err(|e| e.0).unwrap();

    imap_session
        .append_with_flags(
            mailbox,
            eml.write_to_vec().unwrap(),
            &[imap::types::Flag::Seen],
        )
        .unwrap();
}

fn get_builder_from_parser<'a>(message: &'a Message) -> MessageBuilder<'a> {
    let mut eml = MessageBuilder::new().text_body(text_body(message));
    eml = copy_headers(eml, message);
    eml = copy_attachments(eml, message);
    eml
}

fn handle_put_email_on_imap_server(
    eml: &MessageBuilder,
    message: &Message,
    matches: &clap::ArgMatches,
) {
    match (
        matches.get_one::<String>("put-on-imap"),
        matches.get_one::<String>("server"),
        matches
            .get_one::<String>("port")
            .unwrap_or(&String::from("933"))
            .parse::<u16>(),
        matches.get_one::<String>("user"),
        matches.get_one::<String>("password"),
        matches.get_flag("generate-html"),
    ) {
        (Some(mailbox), Some(server), Ok(port), Some(user), Some(pass), generate_html) => {
            let mut eml_to_store = eml.clone();
            if generate_html {
                eml_to_store = eml_to_store.html_body(text_body_as_html(message, None));
            };
            put_email_on_imap_server(eml_to_store, mailbox, server, port, user, pass);
        }
        (None, _, _, _, _, _) => (),
        (_, _, _, _, _, _) => panic!("Missing arguments for put-on-imap"),
    }
}

fn get_file_name(attachment: &MessagePart) -> String {
    let mut result = String::new();
    attachment.headers().iter().for_each(|header| {
        if header.name == HeaderName::Rfc(RfcHeader::ContentDisposition) {
            if let HeaderValue::ContentType(content_type) = &header.value {
                result = content_type.attribute("filename").unwrap().to_owned();
            };
        }
    });
    result
}

fn get_content_type(attachment: &MessagePart) -> String {
    let mut result = String::new();
    attachment.headers().iter().for_each(|header| {
        if header.name == HeaderName::Rfc(RfcHeader::ContentType) {
            if let HeaderValue::ContentType(content_type) = &header.value {
                result.push_str(content_type.ctype().to_owned().as_str());
                if let Some(subtype) = content_type.subtype() {
                    result.push('/');
                    result.push_str(subtype);
                }
            };
        }
    });
    result
}

fn copy_attachments<'a>(mut dest: MessageBuilder<'a>, source: &'a Message) -> MessageBuilder<'a> {
    for attachment in source.attachments() {
        let content_type = get_content_type(attachment);
        let file_name = get_file_name(attachment);

        match &attachment.body {
            PartType::Binary(body) => {
                dest = dest.binary_attachment(content_type, file_name, body.as_ref());
            }
            PartType::Text(body) => {
                dest = dest.text_attachment(content_type, file_name, body.as_ref());
            }
            _ => (),
        }
    }
    dest
}
