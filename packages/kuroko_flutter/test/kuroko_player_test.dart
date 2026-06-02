import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kuroko_flutter/kuroko_flutter.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const playerChannel = MethodChannel('kuroko_flutter/player');
  const eventsChannel = MethodChannel('kuroko_flutter/events');

  late List<MethodCall> playerCalls;

  setUp(() {
    playerCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(playerChannel, (MethodCall call) async {
      playerCalls.add(call);
      return switch (call.method) {
        'create' => 7,
        'dispose' => null,
        _ => null,
      };
    });
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(eventsChannel, (MethodCall call) async {
      return null;
    });
  });

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(playerChannel, null);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(eventsChannel, null);
  });

  test('default player lets native choose output mode', () async {
    final player = KurokoPlayer();

    expect(await player.ensureCreated(), 7);

    final createCall = playerCalls.singleWhere(
      (MethodCall call) => call.method == 'create',
    );
    expect(createCall.arguments, isA<Map<Object?, Object?>>());
    expect(createCall.arguments as Map<Object?, Object?>, isEmpty);

    await player.dispose();
  });

  test('apple EDR output mode is passed to native create', () async {
    final player = KurokoPlayer(
      outputMode: KurokoOutputMode.appleEdr,
      edrHeadroom: 4.0,
    );

    expect(await player.ensureCreated(), 7);

    final createCall = playerCalls.singleWhere(
      (MethodCall call) => call.method == 'create',
    );
    final arguments = createCall.arguments as Map<Object?, Object?>;
    expect(arguments['outputMode'], KurokoOutputMode.appleEdr.nativeValue);
    expect(arguments['edrHeadroom'], 4.0);

    await player.dispose();
  });

  test('external subtitle add returns native track id', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(playerChannel, (MethodCall call) async {
      playerCalls.add(call);
      return switch (call.method) {
        'create' => 7,
        'addExternalSubtitle' => 1000001,
        'dispose' => null,
        _ => null,
      };
    });
    final player = KurokoPlayer();

    final trackId = await player.addExternalSubtitle('/tmp/subs.srt');

    expect(trackId, 1000001);
    final call = playerCalls.singleWhere(
      (MethodCall call) => call.method == 'addExternalSubtitle',
    );
    expect(call.arguments, <String, Object?>{
      'playerId': 7,
      'uri': '/tmp/subs.srt',
    });

    await player.dispose();
  });

  test('external subtitle remove forwards track id', () async {
    final player = KurokoPlayer();

    await player.removeSubtitleTrack(1000001);

    final call = playerCalls.singleWhere(
      (MethodCall call) => call.method == 'removeSubtitleTrack',
    );
    expect(call.arguments, <String, Object?>{
      'playerId': 7,
      'trackId': 1000001,
    });

    await player.dispose();
  });

  test('track selection methods forward nullable track ids', () async {
    final player = KurokoPlayer();

    await player.selectAudioTrack(2);
    await player.selectSubtitleTrack(null);

    final audioCall = playerCalls.singleWhere(
      (MethodCall call) => call.method == 'selectAudioTrack',
    );
    expect(audioCall.arguments, <String, Object?>{
      'playerId': 7,
      'trackId': 2,
    });

    final subtitleCall = playerCalls.singleWhere(
      (MethodCall call) => call.method == 'selectSubtitleTrack',
    );
    expect(subtitleCall.arguments, <String, Object?>{
      'playerId': 7,
      'trackId': null,
    });

    await player.dispose();
  });

  test('tracks query parses native track list', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(playerChannel, (MethodCall call) async {
      playerCalls.add(call);
      return switch (call.method) {
        'create' => 7,
        'tracks' => <Map<String, Object?>>[
            <String, Object?>{
              'id': 0,
              'kind': 0,
              'source': 0,
              'selected': true,
              'canRemove': false,
              'title': 'Main video',
              'language': null,
              'codec': 'hevc',
            },
            <String, Object?>{
              'id': 1000001,
              'kind': 2,
              'source': 1,
              'selected': true,
              'canRemove': true,
              'title': 'subs.srt',
              'language': 'jpn',
              'codec': 'subrip',
            },
          ],
        'dispose' => null,
        _ => null,
      };
    });
    final player = KurokoPlayer();

    final tracks = await player.tracks();

    expect(tracks, hasLength(2));
    expect(tracks.first.kind, KurokoTrackKind.video);
    expect(tracks.first.source, KurokoTrackSource.embedded);
    expect(tracks.first.selected, isTrue);
    expect(tracks.last.kind, KurokoTrackKind.subtitle);
    expect(tracks.last.source, KurokoTrackSource.external);
    expect(tracks.last.canRemove, isTrue);
    expect(tracks.last.title, 'subs.srt');

    await player.dispose();
  });

  test('player event parses track list and selection', () {
    final event = KurokoPlayerEvent.fromMap(<String, Object?>{
      'playerId': 7,
      'kind': KurokoEventKind.trackSelectionChanged.index,
      'state': KurokoPlaybackState.ready.index,
      'durationMicros': 0,
      'positionMicros': 0,
      'buffering': false,
      'video': <String, Object?>{},
      'tracks': <String, Object?>{'video': 1, 'audio': 1, 'subtitle': 1},
      'trackSelection': <String, Object?>{
        'video': 0,
        'audio': -1,
        'subtitle': 1000001,
      },
      'trackList': <Map<String, Object?>>[
        <String, Object?>{
          'id': 1000001,
          'kind': 2,
          'source': 1,
          'selected': true,
          'canRemove': true,
          'title': 'subs.ass',
          'language': null,
          'codec': 'ass',
        },
      ],
      'status': 0,
    });

    expect(event.kind, KurokoEventKind.trackSelectionChanged);
    expect(event.trackSelection.video, 0);
    expect(event.trackSelection.audio, isNull);
    expect(event.trackSelection.subtitle, 1000001);
    expect(event.trackList.single.isExternal, isTrue);
    expect(event.trackList.single.canRemove, isTrue);
  });
}
