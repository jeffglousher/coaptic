use coaptic::link_format::{self, Error, Extension, Metadata, SizeEstimate};

#[test]
fn discovery_examples_keep_borrowed_metadata_and_unknown_attributes() {
    let document = r#"</sensors>;ct=40;title="Sensor Index",</sensors/temp>;rt="temperature-c";if="sensor";ct="0  50";sz=1024;obs;vendor="alpha,beta""#;
    let mut links = link_format::parse(document);
    let index = links.next().unwrap().unwrap();
    assert_eq!(index.target(), "/sensors");
    assert_eq!(index.content_formats().collect::<Vec<_>>(), [40]);
    let temperature = links.next().unwrap().unwrap();
    assert_eq!(
        temperature.resource_types().collect::<Vec<_>>(),
        ["temperature-c"]
    );
    assert_eq!(temperature.interfaces().collect::<Vec<_>>(), ["sensor"]);
    assert_eq!(temperature.content_formats().collect::<Vec<_>>(), [0, 50]);
    assert_eq!(temperature.size_estimate(), Some(SizeEstimate::Bytes(1024)));
    assert!(temperature.observable());
    assert_eq!(temperature.value("vendor").unwrap().encoded(), "alpha,beta");
    assert!(links.next().is_none());
}

#[test]
fn quoted_delimiters_escapes_and_utf8_do_not_split_links() {
    let link = link_format::parse(r#"</a,b>;title="A \"quote\"; café\\",<next>"#)
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(link.target(), "/a,b");
    let title = link.value("title").unwrap();
    assert_eq!(title.chars().collect::<String>(), "A \"quote\"; café\\");
    let mut out = [0x55; 64];
    let len = title.copy_into(&mut out).unwrap();
    assert_eq!(&out[..len], "A \"quote\"; café\\".as_bytes());
    let mut short = [0x55; 2];
    assert_eq!(title.copy_into(&mut short), Err(Error::OutputTooSmall));
    assert_eq!(short, [0x55; 2]);
}

#[test]
fn oversized_cardinal_is_valid_and_not_wrapped() {
    let link = link_format::parse("</large>;sz=184467440737095516160000")
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(link.size_estimate(), Some(SizeEstimate::Larger));
    let link = link_format::parse("</large>;sz=18446744073709551615")
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(link.size_estimate(), Some(SizeEstimate::Bytes(u64::MAX)));
}

#[test]
fn observe_values_and_duplicates_are_ignored_as_required() {
    let link = link_format::parse(r#"</a>;obs="ignored";obs=1;obs"#)
        .next()
        .unwrap()
        .unwrap();
    assert!(link.observable());
    assert_eq!(link.attributes().count(), 3);
}

#[test]
fn attribute_names_are_matched_without_case_but_values_are_preserved() {
    let link = link_format::parse(r#"</a>;RT="urn:example:Temperature";CT=50;OBS;Vendor=Value"#)
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        link.resource_types().collect::<Vec<_>>(),
        ["urn:example:Temperature"]
    );
    assert_eq!(link.content_formats().collect::<Vec<_>>(), [50]);
    assert!(link.observable());
    assert_eq!(link.value("vendor").unwrap().encoded(), "Value");
    assert_eq!(
        link_format::parse("</a>;RT=a;rt=b").next(),
        Some(Err(Error::DuplicateAttribute))
    );
    assert_eq!(
        link_format::parse("</a>;HREF=/b").next(),
        Some(Err(Error::ReservedAttribute))
    );
}

#[test]
fn repeated_extensions_and_content_formats_are_retained() {
    let link = link_format::parse("</a>;ct=0;ct=50;tag=a;tag=b")
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(link.content_formats().collect::<Vec<_>>(), [0, 50]);
    assert_eq!(link.attributes().filter(|a| a.name == "tag").count(), 2);
    assert_eq!(link.value("tag").unwrap().encoded(), "a");
}

#[test]
fn invalid_core_attributes_are_refused_before_yielding_link() {
    for input in [
        "</a>;rt",
        "</a>;if",
        "</a>;sz",
        "</a>;ct",
        "</a>;ct=65536",
        "</a>;sz=01",
        "</a>;sz=-1",
        "</a>;ct=00",
        "</a>;ct=-1",
        r#"</a>;sz="1""#,
        r#"</a>;rt="""#,
        r#"</a>;if=" x""#,
        r#"</a>;rt="x ""#,
        r#"</a>;ct="0 ""#,
        r#"</a>;ct="0 x""#,
        r#"</a>;ct="""#,
        r#"</a>;rt="sensor\"x""#,
    ] {
        assert_eq!(
            link_format::parse(input).next(),
            Some(Err(Error::Syntax)),
            "{input}"
        );
    }
    for input in ["</a>;rt=a;rt=b", "</a>;if=a;if=b", "</a>;sz=1;sz=2"] {
        assert_eq!(
            link_format::parse(input).next(),
            Some(Err(Error::DuplicateAttribute))
        );
    }
    assert_eq!(
        link_format::parse("</a>;href=/x").next(),
        Some(Err(Error::ReservedAttribute))
    );
}

#[test]
fn parser_rejects_bad_framing_and_fuses_after_error() {
    for input in [
        "x",
        "<",
        "</a",
        "</a>,",
        "</a>,,</b>",
        "</a> ",
        " </a>",
        "</a>;",
        "</a>;=x",
        "</a>;x=",
        "</a>;x=\"",
        "</a>;x=\"a\\",
        "</a>;x=\"a\n\"",
        "</a>;x=\"a\r\"",
        "</a>;x=\"b\"garbage",
        "</a b>",
        "</a%>",
        "</a%1>",
        "</a%xy>",
        "</café>",
    ] {
        let mut links = link_format::parse(input);
        let first = links.next().unwrap();
        // A malformed later link can follow an independently valid first link.
        if first.is_ok() {
            assert!(links.next().unwrap().is_err(), "{input}");
        }
        assert!(links.next().is_none(), "{input}");
        assert!(links.next().is_none(), "{input}");
    }
    assert!(link_format::parse("").next().is_none());
    assert_eq!(
        link_format::parse("<>").next().unwrap().unwrap().target(),
        ""
    );
}

#[test]
fn relation_lists_accept_absolute_uri_and_multiple_spaces() {
    let link = link_format::parse(
        r#"</a>;rt="sensor  urn:example:Temperature";if="https://example.org/a%20b""#,
    )
    .next()
    .unwrap()
    .unwrap();
    assert_eq!(
        link.resource_types().collect::<Vec<_>>(),
        ["sensor", "urn:example:Temperature"]
    );
    assert_eq!(
        link.interfaces().collect::<Vec<_>>(),
        ["https://example.org/a%20b"]
    );
}

#[test]
fn metadata_round_trip_and_exact_capacity() {
    let extensions = [
        Extension {
            name: "title",
            value: Some("Kitchen \"temperature\" \\ °C"),
        },
        Extension {
            name: "custom",
            value: None,
        },
        Extension {
            name: "empty",
            value: Some(""),
        },
    ];
    let metadata = Metadata {
        resource_types: Some("temperature-c sensor"),
        interfaces: Some("sensor"),
        content_formats: &[0, 50, u16::MAX],
        size_estimate: Some(u64::MAX),
        observable: true,
        extensions: &extensions,
    };
    let mut bytes = [0xaa; 512];
    let len = link_format::write_link("/a%20b", metadata, &mut bytes).unwrap();
    assert_eq!(len, metadata.encoded_len().unwrap() + "/a%20b".len() + 2);
    let parsed = link_format::parse(core::str::from_utf8(&bytes[..len]).unwrap())
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        parsed.resource_types().collect::<Vec<_>>(),
        ["temperature-c", "sensor"]
    );
    assert_eq!(
        parsed.content_formats().collect::<Vec<_>>(),
        [0, 50, u16::MAX]
    );
    assert_eq!(
        parsed.value("title").unwrap().chars().collect::<String>(),
        extensions[0].value.unwrap()
    );
    assert!(parsed.observable());
    assert!(
        parsed
            .attributes()
            .find(|a| a.name == "custom")
            .unwrap()
            .value
            .is_none()
    );
    assert_eq!(parsed.value("empty").unwrap().encoded(), "");
    let mut exact = vec![0; len];
    assert_eq!(
        link_format::write_link("/a%20b", metadata, &mut exact),
        Ok(len)
    );
    assert_eq!(exact, bytes[..len]);
    for capacity in 0..len {
        let mut short = vec![0xaa; capacity];
        assert_eq!(
            link_format::write_link("/a%20b", metadata, &mut short),
            Err(Error::OutputTooSmall)
        );
        assert!(short.iter().all(|b| *b == 0xaa));
    }
}

#[test]
fn metadata_append_and_invalid_input_leave_output_unchanged() {
    let metadata = Metadata {
        observable: true,
        ..Metadata::default()
    };
    let mut bytes = [0xaa; 4];
    assert_eq!(metadata.write_into(&mut bytes), Ok(4));
    assert_eq!(&bytes, b";obs");
    let mut short = [0xaa; 3];
    assert_eq!(metadata.write_into(&mut short), Err(Error::OutputTooSmall));
    assert_eq!(short, [0xaa; 3]);
    for metadata in [
        Metadata {
            resource_types: Some("bad value!"),
            ..Metadata::default()
        },
        Metadata {
            extensions: &[Extension {
                name: "rt",
                value: Some("x"),
            }],
            ..Metadata::default()
        },
        Metadata {
            extensions: &[Extension {
                name: "x",
                value: Some("\n"),
            }],
            ..Metadata::default()
        },
        Metadata {
            extensions: &[Extension {
                name: "",
                value: None,
            }],
            ..Metadata::default()
        },
    ] {
        let mut bytes = [0xaa; 128];
        assert!(link_format::write_link("/a", metadata, &mut bytes).is_err());
        assert_eq!(bytes, [0xaa; 128]);
    }
}
