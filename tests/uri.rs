use coaptic::uri::{Error, Host, Transport, Uri};

#[test]
fn transport_and_authority_are_explicit() {
    let uri = Uri::parse("CoApS://Example.COM:5688/a").unwrap();
    assert_eq!(uri.transport(), Transport::Dtls);
    assert_eq!(uri.authority(), "Example.COM:5688");
    assert_eq!(uri.port(), 5688);
    assert!(matches!(uri.host(), Host::Name(name) if name.encoded()=="Example.COM"));
    assert_eq!(Uri::parse("coap://host:").unwrap().port(), 5683);
    assert_eq!(Uri::parse("coaps://host").unwrap().port(), 5684);
    assert_eq!(Uri::parse("coap://host:0").unwrap().port(), 0);
}

#[test]
fn split_before_decoding_preserves_option_identity() {
    let uri = Uri::parse("coap://host/a%2Fb//%E2%82%AC?x=a%26b&&plus=+").unwrap();
    let mut output = [0; 32];
    let mut path = uri.path_segments();
    assert_eq!(path.next().unwrap().decode(&mut output).unwrap(), "a/b");
    assert_eq!(path.next().unwrap().decode(&mut output).unwrap(), "");
    assert_eq!(path.next().unwrap().decode(&mut output).unwrap(), "€");
    assert!(path.next().is_none());
    let mut query = uri.query_arguments();
    assert_eq!(query.next().unwrap().decode(&mut output).unwrap(), "x=a&b");
    assert_eq!(query.next().unwrap().decode(&mut output).unwrap(), "");
    assert_eq!(query.next().unwrap().decode(&mut output).unwrap(), "plus=+");
}

#[test]
fn empty_path_query_and_trailing_segments_are_distinct() {
    for text in ["coap://host", "coap://host/"] {
        let uri = Uri::parse(text).unwrap();
        assert_eq!(uri.path_segments().count(), 0);
        assert_eq!(uri.query_arguments().count(), 0);
    }
    assert_eq!(
        Uri::parse("coap://host?")
            .unwrap()
            .query_arguments()
            .count(),
        1
    );
    assert_eq!(
        Uri::parse("coap://host//").unwrap().path_segments().count(),
        2
    );
    assert_eq!(
        Uri::parse("coap://host/a/")
            .unwrap()
            .path_segments()
            .count(),
        2
    );
}

#[test]
fn ipv6_zone_remains_caller_owned() {
    let uri = Uri::parse("coap://[fe80::1%25eth%30]:61616/a").unwrap();
    let Host::Ipv6 { address, zone } = uri.host() else {
        panic!("IPv6")
    };
    assert_eq!(address, "fe80::1".parse::<core::net::Ipv6Addr>().unwrap());
    assert_eq!(zone.unwrap().decode(&mut [0; 8]).unwrap(), "eth0");
    assert_eq!(uri.authority(), "[fe80::1%25eth%30]:61616");
    assert_eq!(uri.port(), 61616);
}

#[test]
fn malformed_and_unresolved_inputs_refuse_then_valid_input_recovers() {
    for input in [
        "/relative",
        "https://host",
        "coap://",
        "coap://user@host",
        "coap://host:65536",
        "coap://host:-1",
        "coap://[::1",
        "coap://[not-ip]",
        "coap://[::1]junk",
        "coap://[fe80::1%eth0]",
        "coap://[fe80::1%25]",
        "coap://host/a#f",
        "coap://host/a b",
        "coap://host/€",
        "coap://host/%",
        "coap://host/%0g",
        "coap://host/%FF",
        "coap://host/%C0%80",
        "coap://host/%ED%A0%80",
        "coap://host/%F4%90%80%80",
    ] {
        assert!(Uri::parse(input).is_err(), "{input}");
    }
    for input in ["coap://host/.", "coap://host/%2e%2E", "coap://host/a/../b"] {
        assert_eq!(Uri::parse(input), Err(Error::DotSegment));
    }
    assert!(Uri::parse("coap://host/%252e").is_ok());
    assert!(Uri::parse("coap://host/a?x=../").is_ok());
}

#[test]
fn decoding_capacity_failure_is_atomic() {
    let uri = Uri::parse("coap://host/%61bc").unwrap();
    let component = uri.path_segments().next().unwrap();
    assert_eq!(component.decoded_len(), 3);
    let mut short = [0x5a; 2];
    assert_eq!(component.decode(&mut short), Err(Error::Capacity));
    assert_eq!(short, [0x5a; 2]);
    assert_eq!(component.decode(&mut [0; 3]).unwrap(), "abc");
}
