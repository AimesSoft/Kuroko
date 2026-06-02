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
  trackSelectionChanged,
}

enum KurokoTrackKind {
  video,
  audio,
  subtitle,
}

enum KurokoTrackSource {
  embedded,
  external,
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

class KurokoTrackSelection {
  const KurokoTrackSelection({
    this.video,
    this.audio,
    this.subtitle,
  });

  factory KurokoTrackSelection.fromMap(Map<dynamic, dynamic>? map) {
    return KurokoTrackSelection(
      video: _trackId(map?['video']),
      audio: _trackId(map?['audio']),
      subtitle: _trackId(map?['subtitle']),
    );
  }

  final int? video;
  final int? audio;
  final int? subtitle;

  static int? _trackId(Object? value) {
    final id = _asInt(value);
    return id >= 0 ? id : null;
  }

  static int _asInt(Object? value) {
    if (value is int) {
      return value;
    }
    if (value is num) {
      return value.toInt();
    }
    return -1;
  }
}

class KurokoTrackInfo {
  const KurokoTrackInfo({
    required this.id,
    required this.kind,
    required this.source,
    required this.selected,
    required this.canRemove,
    this.title,
    this.language,
    this.codec,
  });

  factory KurokoTrackInfo.fromMap(Map<dynamic, dynamic> map) {
    return KurokoTrackInfo(
      id: _asInt(map['id']),
      kind: _trackKindFromIndex(_asInt(map['kind'])),
      source: _trackSourceFromIndex(_asInt(map['source'])),
      selected: map['selected'] == true,
      canRemove: map['canRemove'] == true,
      title: map['title'] as String?,
      language: map['language'] as String?,
      codec: map['codec'] as String?,
    );
  }

  final int id;
  final KurokoTrackKind kind;
  final KurokoTrackSource source;
  final bool selected;
  final bool canRemove;
  final String? title;
  final String? language;
  final String? codec;

  bool get isEmbedded => source == KurokoTrackSource.embedded;
  bool get isExternal => source == KurokoTrackSource.external;

  static int _asInt(Object? value) {
    if (value is int) {
      return value;
    }
    if (value is num) {
      return value.toInt();
    }
    return 0;
  }

  static KurokoTrackKind _trackKindFromIndex(int index) {
    if (index >= 0 && index < KurokoTrackKind.values.length) {
      return KurokoTrackKind.values[index];
    }
    return KurokoTrackKind.video;
  }

  static KurokoTrackSource _trackSourceFromIndex(int index) {
    if (index >= 0 && index < KurokoTrackSource.values.length) {
      return KurokoTrackSource.values[index];
    }
    return KurokoTrackSource.embedded;
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
    required this.trackList,
    required this.trackSelection,
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
      trackList: _trackListFromValue(map['trackList']),
      trackSelection: KurokoTrackSelection.fromMap(
        map['trackSelection'] as Map<dynamic, dynamic>?,
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
  final List<KurokoTrackInfo> trackList;
  final KurokoTrackSelection trackSelection;
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

  static List<KurokoTrackInfo> _trackListFromValue(Object? value) {
    if (value is! List) {
      return const <KurokoTrackInfo>[];
    }
    return value
        .whereType<Map<dynamic, dynamic>>()
        .map(KurokoTrackInfo.fromMap)
        .toList(growable: false);
  }
}
