use zunel_providers::sse::SseBuffer;

#[test]
fn emits_single_data_event() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: hello\n\n");
    assert_eq!(events, vec![Some("hello".to_string())]);
}

#[test]
fn splits_across_chunks() {
    let mut buf = SseBuffer::new();
    assert!(buf.feed(b"data: part").is_empty());
    assert!(buf.feed(b"ial\n").is_empty());
    let events = buf.feed(b"\n");
    assert_eq!(events, vec![Some("partial".to_string())]);
}

#[test]
fn multiple_events_in_one_chunk() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: a\n\ndata: b\n\n");
    assert_eq!(
        events,
        vec![Some("a".to_string()), Some("b".to_string())]
    );
}

#[test]
fn done_sentinel_emits_none() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: [DONE]\n\n");
    assert_eq!(events, vec![None]);
}

#[test]
fn ignores_comments_and_unknown_fields() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b": keepalive\nevent: foo\ndata: value\n\n");
    assert_eq!(events, vec![Some("value".to_string())]);
}

#[test]
fn multiline_data_joins_with_newlines() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: line1\ndata: line2\n\n");
    assert_eq!(events, vec![Some("line1\nline2".to_string())]);
}

#[test]
fn handles_crlf_line_endings() {
    let mut buf = SseBuffer::new();
    let events = buf.feed(b"data: hi\r\n\r\n");
    assert_eq!(events, vec![Some("hi".to_string())]);
}
