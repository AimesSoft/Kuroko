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
}
