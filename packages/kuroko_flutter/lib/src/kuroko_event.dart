enum KurokoPlaybackState {
  idle,
  opening,
  ready,
  playing,
  paused,
  stopped,
  closed,
  error,
}

enum KurokoEventKind {
  none,
  stateChanged,
  durationChanged,
  positionChanged,
  tracksChanged,
  bufferingChanged,
  videoParamsChanged,
  surfaceAttached,
  surfaceDetached,
  error,
}

class KurokoVideoParams {
  const KurokoVideoParams({
    required this.width,
    required this.height,
    required this.primaries,
    required this.transfer,
  });

  factory KurokoVideoParams.fromMap(Map<dynamic, dynamic>? map) {
    return KurokoVideoParams(
      width: _asInt(map?['width']),
      height: _asInt(map?['height']),
      primaries: _asInt(map?['primaries']),
      transfer: _asInt(map?['transfer']),
    );
  }

  final int width;
  final int height;
  final int primaries;
  final int transfer;

  static int _asInt(Object? value) {
    if (value is int) {
      return value;
    }
    if (value is num) {
      return value.toInt();
    }
    return 0;
  }
}

class KurokoTrackCounts {
  const KurokoTrackCounts({
    required this.video,
    required this.audio,
    required this.subtitle,
  });

  factory KurokoTrackCounts.fromMap(Map<dynamic, dynamic>? map) {
    return KurokoTrackCounts(
      video: _asInt(map?['video']),
      audio: _asInt(map?['audio']),
      subtitle: _asInt(map?['subtitle']),
    );
  }

  final int video;
  final int audio;
  final int subtitle;

  static int _asInt(Object? value) {
    if (value is int) {
      return value;
    }
    if (value is num) {
      return value.toInt();
    }
    return 0;
  }
}

class KurokoPlayerEvent {
  const KurokoPlayerEvent({
    required this.playerId,
    required this.kind,
    required this.state,
    required this.duration,
    required this.position,
    required this.buffering,
    required this.video,
    required this.tracks,
    this.status = 0,
  });

  factory KurokoPlayerEvent.fromMap(Map<dynamic, dynamic> map) {
    return KurokoPlayerEvent(
      playerId: _asInt(map['playerId']),
      kind: _eventKindFromIndex(_asInt(map['kind'])),
      state: _stateFromIndex(_asInt(map['state'])),
      duration: Duration(microseconds: _asInt(map['durationMicros'])),
      position: Duration(microseconds: _asInt(map['positionMicros'])),
      buffering: map['buffering'] == true,
      video: KurokoVideoParams.fromMap(map['video'] as Map<dynamic, dynamic>?),
      tracks: KurokoTrackCounts.fromMap(
        map['tracks'] as Map<dynamic, dynamic>?,
      ),
      status: _asInt(map['status']),
    );
  }

  final int playerId;
  final KurokoEventKind kind;
  final KurokoPlaybackState state;
  final Duration duration;
  final Duration position;
  final bool buffering;
  final KurokoVideoParams video;
  final KurokoTrackCounts tracks;
  final int status;

  static int _asInt(Object? value) {
    if (value is int) {
      return value;
    }
    if (value is num) {
      return value.toInt();
    }
    return 0;
  }

  static KurokoEventKind _eventKindFromIndex(int index) {
    if (index >= 0 && index < KurokoEventKind.values.length) {
      return KurokoEventKind.values[index];
    }
    return KurokoEventKind.none;
  }

  static KurokoPlaybackState _stateFromIndex(int index) {
    if (index >= 0 && index < KurokoPlaybackState.values.length) {
      return KurokoPlaybackState.values[index];
    }
    return KurokoPlaybackState.error;
  }
}
