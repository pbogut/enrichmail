use clap::arg;
use comrak::{markdown_to_html, ComrakOptions};
use mail_builder::headers as b_headers;
use mail_builder::headers::HeaderType;
use mail_builder::MessageBuilder;

use base64::{engine::general_purpose, Engine as _};
use mail_parser::{Addr, HeaderName, HeaderValue, Message, MessagePart, PartType, RfcHeader};

fn text_body(message: &Message) -> String {
    message.body_text(0).unwrap().to_owned().to_string()
}

fn text_body_as_html(message: &Message, append: Option<String>) -> String {
    let body = markdown_to_html(text_body(&message).as_str(), &ComrakOptions::default());
    let body_append = match append {
        Some(append) => append,
        None => "".to_owned(),
    };
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

fn transform_address(address: Addr) -> b_headers::address::Address {
    let name = match address.name.clone() {
        Some(name) => Some(name.to_owned()),
        None => None,
    };
    b_headers::address::Address::new_address(name, address.address.as_ref().unwrap().to_owned())
}

fn copy_headers<'a>(source: &'a Message, dest: MessageBuilder<'a>) -> MessageBuilder<'a> {
    let mut new_dest = dest.clone();
    for header in source.headers() {
        let new_header = match header.value().clone() {
            HeaderValue::Address(address) => HeaderType::Address(transform_address(address)),
            HeaderValue::Text(text) => HeaderType::Text(b_headers::text::Text::new(text)),
            HeaderValue::DateTime(datetime) => {
                HeaderType::Date(b_headers::date::Date::new(datetime.to_timestamp()))
            }
            HeaderValue::ContentType(conent_type) => {
                let mut ctype = conent_type.ctype().to_owned();
                if let Some(subtype) = conent_type.subtype() {
                    ctype.push_str("/");
                    ctype.push_str(subtype);
                }
                HeaderType::ContentType(b_headers::content_type::ContentType::new(ctype))
            }
            HeaderValue::AddressList(addresses) => {
                let mut new_addresses = vec![];
                addresses.iter().for_each(|address| {
                    let new_address = transform_address(address.clone());
                    new_addresses.push(new_address);
                });
                HeaderType::Address(b_headers::address::Address::List(new_addresses))
            }
            HeaderValue::Group(group) => todo!("Group not implemented {:?}", group),
            HeaderValue::GroupList(group_list) => {
                todo!("Group list not implemented {:?}", group_list)
            }
            HeaderValue::TextList(text_list) => todo!("Text list not implemented {:?}", text_list),
            HeaderValue::Empty => todo!("Empty not implemented"),
        };
        new_dest = new_dest.header(header.name(), new_header);
    }
    new_dest
}

fn main() {
    let matches = clap::Command::new("cargo")
        .about("Email enrich tool for mutt")
        .args(vec![
            arg!(--genhtml "Generate html body from markdown in text body"),
            arg!(--addpixel <BASE_URL> "Add tracking pixel to html body").requires("genhtml"),
        ])
        .get_matches();

    let stdin = std::io::stdin();
    let mut input = String::new();

    while let Ok(n) = stdin.read_line(&mut input) {
        if n == 0 {
            break;
        }
    }

    let message = Message::parse(input.as_bytes()).unwrap();

    let mut eml = MessageBuilder::new().text_body(text_body(&message));

    eml = copy_headers(&message, eml);
    eml = copy_attachments(&message, eml);

    let append = match matches.get_one::<String>("addpixel") {
        Some(tracking_url) => {
            let encoded_id: String =
                general_purpose::STANDARD_NO_PAD.encode(message.message_id().unwrap().to_owned());

            let pixel_url = format!("{}/image/{}.gif", tracking_url, encoded_id);
            let pixel = format!(
                "<img src=\"{}\" alt=\"Open pixel\" style=\"border: 0px; width: 0px; max-width: 1px;\" />",
                pixel_url
            );
            Some(pixel)
        }
        None => None,
    };

    if matches.get_flag("genhtml") {
        eml = eml.html_body(text_body_as_html(&message, append));
    }
    println!("{}", eml.write_to_string().unwrap());
}

fn get_file_name(attachment: &MessagePart) -> String {
    let mut result = String::new();
    attachment.headers().into_iter().for_each(|header| {
        if let HeaderName::Rfc(RfcHeader::ContentDisposition) = header.name {
            match header.value.clone() {
                HeaderValue::ContentType(content_type) => {
                    result = content_type.attribute("filename").unwrap().to_owned();
                }
                _ => (),
            };
        }
    });
    result
}

fn get_content_type(attachment: &MessagePart) -> String {
    let mut result = String::new();
    attachment.headers().into_iter().for_each(|header| {
        if let HeaderName::Rfc(RfcHeader::ContentType) = header.name {
            match header.value.clone() {
                HeaderValue::ContentType(content_type) => {
                    result.push_str(content_type.ctype().to_owned().as_str());
                    if let Some(subtype) = content_type.subtype() {
                        result.push_str("/");
                        result.push_str(subtype);
                    }
                }
                _ => (),
            };
        }
    });
    result
}

fn copy_attachments<'a>(source: &'a Message, dest: MessageBuilder<'a>) -> MessageBuilder<'a> {
    let mut new_dest = dest.clone();

    source.attachments().for_each(|attachment| {
        let content_type = get_content_type(&attachment);
        let file_name = get_file_name(&attachment);

        match attachment.body.clone() {
            PartType::Binary(body) => {
                new_dest = new_dest
                    .clone()
                    .binary_attachment(content_type, file_name, body);
            }
            PartType::Text(body) => {
                new_dest = new_dest
                    .clone()
                    .text_attachment(content_type, file_name, body);
            }
            _ => (),
        }
    });
    new_dest
}
