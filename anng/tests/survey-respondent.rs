use anng::Message;
use std::{io::Write, time::Duration};
use tracing::Instrument;

#[tokio::test]
async fn basic_survey_response() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let surveyor = anng::protocols::survey0::Surveyor0::listen(c"inproc://basic_survey")
        .await
        .unwrap();
    let respondent = anng::protocols::survey0::Respondent0::dial(c"inproc://basic_survey")
        .await
        .unwrap();

    let surveyor_task = async {
        let mut ctx = surveyor.context();

        let mut survey = Message::with_capacity(100);
        write!(&mut survey, "Who wants to be leader?").unwrap();

        let timeout = Duration::from_secs(1);
        let mut responses = ctx.survey(survey, timeout).await.unwrap();
        let mut response_count = 0;

        while let Some(response) = responses.next().await {
            let msg = response.unwrap();
            response_count += 1;
            tracing::info!(
                "Got response: {:?}",
                std::str::from_utf8(msg.as_slice()).unwrap()
            );
            assert_eq!(msg.as_slice(), b"I volunteer!");
        }

        assert_eq!(response_count, 1);
    }
    .instrument(tracing::info_span!("surveyor"));

    let respondent_task = async {
        let mut ctx = respondent.context();

        let (survey, responder) = ctx.next_survey().await.unwrap();
        tracing::info!(
            "Got survey: {:?}",
            std::str::from_utf8(survey.as_slice()).unwrap()
        );
        assert_eq!(survey.as_slice(), b"Who wants to be leader?");

        {
            let mut response = Message::with_capacity(100);
            write!(&mut response, "I volunteer!").unwrap();
            responder.respond(response).await.unwrap();
        }
    }
    .instrument(tracing::info_span!("respondent"));

    tokio::join!(surveyor_task, respondent_task);
}

#[tokio::test]
async fn multiple_respondents() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let surveyor = anng::protocols::survey0::Surveyor0::listen(c"inproc://multi_survey")
        .await
        .unwrap();

    // Create multiple respondents
    let respondent1 = anng::protocols::survey0::Respondent0::dial(c"inproc://multi_survey")
        .await
        .unwrap();
    let respondent2 = anng::protocols::survey0::Respondent0::dial(c"inproc://multi_survey")
        .await
        .unwrap();
    let respondent3 = anng::protocols::survey0::Respondent0::dial(c"inproc://multi_survey")
        .await
        .unwrap();

    let surveyor_task = async {
        // give respondents some time to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        let mut ctx = surveyor.context();

        let mut survey = Message::with_capacity(100);
        write!(&mut survey, "Status check").unwrap();

        let timeout = Duration::from_secs(1);
        let mut responses = ctx.survey(survey, timeout).await.unwrap();
        let mut response_count = 0;
        let mut received_responses = Vec::new();

        while let Some(response) = responses.next().await {
            let msg = response.unwrap();
            response_count += 1;
            let response_text = std::str::from_utf8(msg.as_slice()).unwrap();
            tracing::info!("Got response {}: {:?}", response_count, response_text);
            received_responses.push(response_text.to_string());
        }

        tracing::info!("Survey completed; total responses: {}", response_count);

        // We should get responses from all 3 respondents
        assert_eq!(
            response_count, 3,
            "Should receive responses from all the respondents"
        );
        assert!(
            response_count <= 3,
            "Should not receive more than 3 responses"
        );
    }
    .instrument(tracing::info_span!("surveyor"));

    let respondent_task =
        async move |id: u32, socket: anng::Socket<anng::protocols::survey0::Respondent0>| {
            let mut ctx = socket.context();

            let (survey, responder) = ctx.next_survey().await.unwrap();
            tracing::info!(
                "Respondent {} got survey: {:?}",
                id,
                std::str::from_utf8(survey.as_slice()).unwrap()
            );

            {
                let mut response = Message::with_capacity(100);
                write!(&mut response, "Respondent {} is healthy", id).unwrap();
                responder.respond(response).await.unwrap();
            }
        };

    tokio::join!(
        biased;
        respondent_task(1, respondent1).instrument(tracing::info_span!("respondent1")),
        respondent_task(2, respondent2).instrument(tracing::info_span!("respondent2")),
        respondent_task(3, respondent3).instrument(tracing::info_span!("respondent3")),
        surveyor_task,
    );
}

#[tokio::test]
async fn no_response() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let surveyor = anng::protocols::survey0::Surveyor0::listen(c"inproc://no_response")
        .await
        .unwrap();
    let respondent = anng::protocols::survey0::Respondent0::dial(c"inproc://no_response")
        .await
        .unwrap();

    let surveyor_task = async {
        let mut ctx = surveyor.context();

        let mut survey = Message::with_capacity(100);
        write!(&mut survey, "Anyone there?").unwrap();

        let timeout = Duration::from_secs(1);
        let mut responses = ctx.survey(survey, timeout).await.unwrap();
        let mut response_count = 0;

        while let Some(response) = responses.next().await {
            let msg = response.unwrap();
            response_count += 1;
            tracing::info!(
                "Unexpected response: {:?}",
                std::str::from_utf8(msg.as_slice()).unwrap()
            );
        }

        tracing::info!("Survey completed");

        // Should receive no responses since respondent doesn't reply
        assert_eq!(response_count, 0);
    }
    .instrument(tracing::info_span!("surveyor"));

    let respondent_task = async {
        let mut ctx = respondent.context();

        let (survey, responder) = ctx.next_survey().await.unwrap();
        tracing::info!(
            "Got survey: {:?}",
            std::str::from_utf8(survey.as_slice()).unwrap()
        );

        // Intentionally drop the responder without responding
        let _ = responder;
    }
    .instrument(tracing::info_span!("respondent"));

    tokio::join!(biased; respondent_task, surveyor_task);
}

#[tokio::test]
async fn cancellation_safety() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let surveyor = anng::protocols::survey0::Surveyor0::listen(c"inproc://cancellation")
        .await
        .unwrap();
    let respondent = anng::protocols::survey0::Respondent0::dial(c"inproc://cancellation")
        .await
        .unwrap();

    let surveyor_task = async {
        let mut ctx = surveyor.context();

        for i in 0..5 {
            let mut survey = Message::with_capacity(100);
            write!(&mut survey, "Survey #{}", i).unwrap();

            let timeout = Duration::from_secs(1);
            let mut responses = ctx.survey(survey, timeout).await.unwrap();

            // Test cancellation safety by only partially consuming responses
            let mut response = None;
            loop {
                tokio::select! {
                    biased;
                    r = responses.next() => {
                        if let Some(r) = r {
                            response = Some(r.unwrap());
                        } else {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_micros(1)) => {
                        // hitting this cancels the .next() future
                    }
                }
            }

            // We should get a response eventually.
            let msg = response.unwrap();
            tracing::info!(
                "Survey {} got response: {:?}",
                i,
                std::str::from_utf8(msg.as_slice()).unwrap()
            );
            assert_eq!(
                msg.as_slice(),
                format!("Response to survey #{}", i).as_bytes()
            );
        }
    }
    .instrument(tracing::info_span!("surveyor"));

    let respondent_task = async {
        let mut ctx = respondent.context();

        for i in 0..5 {
            // Test cancellation safety by using select! with sleep
            // We should _eventually_ get the survey.
            let (survey, responder) = loop {
                tokio::select! {
                    biased;
                    r = ctx.next_survey() => { break r.unwrap(); }
                    _ = tokio::time::sleep(Duration::from_micros(1)) => {}
                }
            };

            tracing::info!(
                "Got survey {}: {:?}",
                i,
                std::str::from_utf8(survey.as_slice()).unwrap()
            );

            {
                let mut response = Message::with_capacity(100);
                write!(&mut response, "Response to survey #{}", i).unwrap();
                responder.respond(response).await.unwrap();
            }
        }
    }
    .instrument(tracing::info_span!("respondent"));

    tokio::join!(surveyor_task, respondent_task);
}
