import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'kuroko_event.dart';

enum KurokoOutputMode {
  sdr(0),
  appleEdr(1);

  const KurokoOutputMode(this.nativeValue);

  final int nativeValue;
}

class KurokoDanmakuTrackInfo {
  const KurokoDanmakuTrackInfo({
    required this.id,
    required this.enabled,
    required this.offset,
    required this.itemCount,
    this.name,
    this.source,
  });

  final int id;
  final bool enabled;
  final Duration offset;
  final int itemCount;
  final String? name;
  final String? source;

  factory KurokoDanmakuTrackInfo.fromMap(Map<dynamic, dynamic> map) {
    return KurokoDanmakuTrackInfo(
      id: (map['id'] as num?)?.toInt() ?? 0,
      enabled: map['enabled'] == true,
      offset:
          Duration(microseconds: (map['offsetMicros'] as num?)?.toInt() ?? 0),
      itemCount: (map['itemCount'] as num?)?.toInt() ?? 0,
      name: map['name'] as String?,
      source: map['source'] as String?,
    );
  }
}

class KurokoPlayer {
  KurokoPlayer({
    this.outputMode,
    this.edrHeadroom,
  }) {
    _eventSubscription ??= _events.receiveBroadcastStream().listen(
      _dispatchNativeEvent,
      onError: (Object error, StackTrace stackTrace) {
        debugPrint('KurokoPlayer event stream error: $error');
      },
    );
  }

  static const MethodChannel _channel = MethodChannel('kuroko_flutter/player');
  static const EventChannel _events = EventChannel('kuroko_flutter/events');
  static const int windowOverlayViewId = -1;
  static final Map<int, StreamController<KurokoPlayerEvent>> _controllers =
      <int, StreamController<KurokoPlayerEvent>>{};
  static StreamSubscription<dynamic>? _eventSubscription;

  int? _id;
  Future<int>? _createFuture;
  bool _disposed = false;

  final KurokoOutputMode? outputMode;
  final double? edrHeadroom;

  int? get id => _id;

  Stream<KurokoPlayerEvent> get events async* {
    final playerId = await ensureCreated();
    yield* _controllerFor(playerId).stream;
  }

  Future<int> ensureCreated() {
    if (_disposed) {
      throw StateError('KurokoPlayer has been disposed.');
    }
    final existing = _id;
    if (existing != null) {
      return Future<int>.value(existing);
    }
    return _createFuture ??= _create();
  }

  Future<void> open(String uri) async {
    final playerId = await ensureCreated();
    await _invoke('open', <String, Object?>{'playerId': playerId, 'uri': uri});
  }

  Future<void> play() async {
    await _invokeForPlayer('play');
  }

  Future<void> pause() async {
    await _invokeForPlayer('pause');
  }

  Future<void> stop() async {
    await _invokeForPlayer('stop');
  }

  Future<void> close() async {
    await _invokeForPlayer('close');
  }

  Future<void> seek(Duration position) async {
    final playerId = await ensureCreated();
    await _invoke('seek', <String, Object?>{
      'playerId': playerId,
      'positionMicros': position.inMicroseconds,
    });
  }

  Future<void> setPlaybackRate(double rate) async {
    final playerId = await ensureCreated();
    await _invoke('setPlaybackRate', <String, Object?>{
      'playerId': playerId,
      'rate': rate,
    });
  }

  Future<int> addExternalSubtitle(String uri) async {
    final playerId = await ensureCreated();
    final trackId = await _channel.invokeMethod<int>(
      'addExternalSubtitle',
      <String, Object?>{'playerId': playerId, 'uri': uri},
    );
    if (trackId == null) {
      throw StateError('Kuroko external subtitle add returned no track id.');
    }
    return trackId;
  }

  Future<void> removeSubtitleTrack(int trackId) async {
    final playerId = await ensureCreated();
    await _invoke('removeSubtitleTrack', <String, Object?>{
      'playerId': playerId,
      'trackId': trackId,
    });
  }

  Future<void> loadDanmakuFile(String uri) async {
    final playerId = await ensureCreated();
    await _invoke('loadDanmakuFile', <String, Object?>{
      'playerId': playerId,
      'uri': uri,
    });
  }

  Future<void> loadDanmakuJson(String json) async {
    final playerId = await ensureCreated();
    await _invoke('loadDanmakuJson', <String, Object?>{
      'playerId': playerId,
      'json': json,
    });
  }

  Future<int> addDanmakuTrackFile(
    String uri, {
    String? name,
    Duration offset = Duration.zero,
  }) async {
    final playerId = await ensureCreated();
    final trackId = await _channel.invokeMethod<int>(
      'addDanmakuTrackFile',
      <String, Object?>{
        'playerId': playerId,
        'uri': uri,
        if (name != null) 'name': name,
        'offsetMicros': offset.inMicroseconds,
      },
    );
    if (trackId == null || trackId <= 0) {
      throw StateError('Kuroko danmaku track add returned no track id.');
    }
    return trackId;
  }

  Future<int> addDanmakuTrackJson(
    String json, {
    String? name,
    Duration offset = Duration.zero,
  }) async {
    final playerId = await ensureCreated();
    final trackId = await _channel.invokeMethod<int>(
      'addDanmakuTrackJson',
      <String, Object?>{
        'playerId': playerId,
        'json': json,
        if (name != null) 'name': name,
        'offsetMicros': offset.inMicroseconds,
      },
    );
    if (trackId == null || trackId <= 0) {
      throw StateError('Kuroko danmaku track add returned no track id.');
    }
    return trackId;
  }

  Future<void> removeDanmakuTrack(int trackId) async {
    final playerId = await ensureCreated();
    await _invoke('removeDanmakuTrack', <String, Object?>{
      'playerId': playerId,
      'trackId': trackId,
    });
  }

  Future<void> setDanmakuTrackEnabled(int trackId, bool enabled) async {
    final playerId = await ensureCreated();
    await _invoke('setDanmakuTrackEnabled', <String, Object?>{
      'playerId': playerId,
      'trackId': trackId,
      'enabled': enabled,
    });
  }

  Future<void> setDanmakuTrackOffset(int trackId, Duration offset) async {
    final playerId = await ensureCreated();
    await _invoke('setDanmakuTrackOffset', <String, Object?>{
      'playerId': playerId,
      'trackId': trackId,
      'offsetMicros': offset.inMicroseconds,
    });
  }

  Future<void> setDanmakuGlobalOffset(Duration offset) async {
    final playerId = await ensureCreated();
    await _invoke('setDanmakuGlobalOffset', <String, Object?>{
      'playerId': playerId,
      'offsetMicros': offset.inMicroseconds,
    });
  }

  Future<List<KurokoDanmakuTrackInfo>> danmakuTracks() async {
    final playerId = await ensureCreated();
    final rawTracks = await _channel.invokeMethod<List<dynamic>>(
      'danmakuTracks',
      <String, Object?>{'playerId': playerId},
    );
    if (rawTracks == null) {
      return const <KurokoDanmakuTrackInfo>[];
    }
    return rawTracks
        .whereType<Map<dynamic, dynamic>>()
        .map(KurokoDanmakuTrackInfo.fromMap)
        .toList(growable: false);
  }

  Future<void> clearDanmaku() async {
    await _invokeForPlayer('clearDanmaku');
  }

  Future<void> setDanmakuEnabled(bool enabled) async {
    final playerId = await ensureCreated();
    await _invoke('setDanmakuEnabled', <String, Object?>{
      'playerId': playerId,
      'enabled': enabled,
    });
  }

  Future<void> setDanmakuConfig({
    bool? enabled,
    double? fontSize,
    double? opacity,
    double? displayArea,
    double? scrollDurationSeconds,
    double? scrollSpeedFactor,
    double? trackGapRatio,
    double? outlineWidth,
    double? shadowOffsetX,
    double? shadowOffsetY,
    int? shadowStyle,
    String? customFontFamily,
    String? customFontFilePath,
    bool? mergeDuplicates,
    bool? allowStacking,
    bool? allowScrollOverwrite,
    int? maxQuantity,
    int? maxLinesPerMode,
    bool? blockTop,
    bool? blockBottom,
    bool? blockScroll,
    List<String>? blockWords,
  }) async {
    final playerId = await ensureCreated();
    await _invoke('setDanmakuConfig', <String, Object?>{
      'playerId': playerId,
      if (enabled != null) 'enabled': enabled,
      if (fontSize != null) 'fontSize': fontSize,
      if (opacity != null) 'opacity': opacity,
      if (displayArea != null) 'displayArea': displayArea,
      if (scrollDurationSeconds != null)
        'scrollDurationSeconds': scrollDurationSeconds,
      if (scrollSpeedFactor != null) 'scrollSpeedFactor': scrollSpeedFactor,
      if (trackGapRatio != null) 'trackGapRatio': trackGapRatio,
      if (outlineWidth != null) 'outlineWidth': outlineWidth,
      if (shadowOffsetX != null) 'shadowOffsetX': shadowOffsetX,
      if (shadowOffsetY != null) 'shadowOffsetY': shadowOffsetY,
      if (shadowStyle != null) 'shadowStyle': shadowStyle,
      if (customFontFamily != null) 'customFontFamily': customFontFamily,
      if (customFontFilePath != null) 'customFontFilePath': customFontFilePath,
      if (mergeDuplicates != null) 'mergeDuplicates': mergeDuplicates,
      if (allowStacking != null) 'allowStacking': allowStacking,
      if (allowScrollOverwrite != null)
        'allowScrollOverwrite': allowScrollOverwrite,
      if (maxQuantity != null) 'maxQuantity': maxQuantity,
      if (maxLinesPerMode != null) 'maxLinesPerMode': maxLinesPerMode,
      if (blockTop != null) 'blockTop': blockTop,
      if (blockBottom != null) 'blockBottom': blockBottom,
      if (blockScroll != null) 'blockScroll': blockScroll,
      if (blockWords != null) 'blockWordsJson': jsonEncode(blockWords),
    });
  }

  Future<void> selectAudioTrack(int? trackId) async {
    final playerId = await ensureCreated();
    await _invoke('selectAudioTrack', <String, Object?>{
      'playerId': playerId,
      'trackId': trackId,
    });
  }

  Future<void> selectSubtitleTrack(int? trackId) async {
    final playerId = await ensureCreated();
    await _invoke('selectSubtitleTrack', <String, Object?>{
      'playerId': playerId,
      'trackId': trackId,
    });
  }

  Future<List<KurokoTrackInfo>> tracks() async {
    final playerId = await ensureCreated();
    final rawTracks = await _channel.invokeMethod<List<dynamic>>(
      'tracks',
      <String, Object?>{'playerId': playerId},
    );
    if (rawTracks == null) {
      return const <KurokoTrackInfo>[];
    }
    return rawTracks
        .whereType<Map<dynamic, dynamic>>()
        .map(KurokoTrackInfo.fromMap)
        .toList(growable: false);
  }

  Future<void> attachView(int viewId) async {
    final playerId = await ensureCreated();
    await _invoke('attachView', <String, Object?>{
      'playerId': playerId,
      'viewId': viewId,
    });
  }

  Future<void> detachView(int viewId) async {
    final playerId = _id;
    if (playerId == null || _disposed) {
      return;
    }
    await _invoke('detachView', <String, Object?>{
      'playerId': playerId,
      'viewId': viewId,
    });
  }

  Future<int> attachWindowOverlay() async {
    final playerId = await ensureCreated();
    final viewId = await _channel.invokeMethod<int>(
      'attachOverlay',
      <String, Object?>{'playerId': playerId},
    );
    return viewId ?? windowOverlayViewId;
  }

  Future<void> detachWindowOverlay({int? generation}) async {
    final playerId = _id;
    if (playerId == null || _disposed) {
      return;
    }
    await _invoke('detachOverlay', <String, Object?>{
      'playerId': playerId,
      if (generation != null) 'generation': generation,
    });
  }

  Future<void> setWindowOverlayFrame({
    required Rect frame,
    required bool visible,
    required int generation,
    String? debugLabel,
  }) async {
    await _invoke('setOverlayFrame', <String, Object?>{
      'viewId': windowOverlayViewId,
      'generation': generation,
      'x': frame.left,
      'y': frame.top,
      'width': frame.width,
      'height': frame.height,
      'visible': visible,
      if (debugLabel != null) 'debugLabel': debugLabel,
    });
  }

  Future<void> dispose() async {
    if (_disposed) {
      return;
    }
    _disposed = true;
    final playerId = _id;
    _id = null;
    _createFuture = null;
    if (playerId == null) {
      return;
    }
    await _invoke('dispose', <String, Object?>{'playerId': playerId});
    final controller = _controllers.remove(playerId);
    await controller?.close();
  }

  Future<int> _create() async {
    final playerId = await _channel.invokeMethod<int>(
      'create',
      <String, Object?>{
        if (outputMode case final mode?) 'outputMode': mode.nativeValue,
        if (edrHeadroom case final headroom?) 'edrHeadroom': headroom,
      },
    );
    if (playerId == null || playerId <= 0) {
      throw StateError('Kuroko presenter creation failed.');
    }
    _id = playerId;
    _controllerFor(playerId);
    return playerId;
  }

  Future<void> _invokeForPlayer(String method) async {
    final playerId = await ensureCreated();
    await _invoke(method, <String, Object?>{'playerId': playerId});
  }

  Future<void> _invoke(String method, Map<String, Object?> arguments) async {
    await _channel.invokeMethod<void>(method, arguments);
  }

  static StreamController<KurokoPlayerEvent> _controllerFor(int playerId) {
    return _controllers.putIfAbsent(
      playerId,
      () => StreamController<KurokoPlayerEvent>.broadcast(),
    );
  }

  static void _dispatchNativeEvent(dynamic rawEvent) {
    if (rawEvent is! Map) {
      return;
    }
    final event = KurokoPlayerEvent.fromMap(rawEvent);
    final controller = _controllers[event.playerId];
    controller?.add(event);
  }
}
