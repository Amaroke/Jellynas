ROLE = "MusicDJ"
ALONE_SECONDS = 60
IDLE_SECONDS = 300


def play_refusal(is_dj, user_channel, bot_channel, busy):
    if not is_dj:
        return f"Only a member with the {ROLE} role can command the music bot."
    if user_channel is None:
        return "Join a voice channel first."
    if bot_channel is not None and bot_channel != user_channel and busy:
        return "The music bot is playing in another voice channel."
    return None


def control_refusal(is_dj, user_channel, bot_channel):
    if not is_dj:
        return f"Only a member with the {ROLE} role can command the music bot."
    if bot_channel is None:
        return "The music bot is not in a voice channel."
    if user_channel != bot_channel:
        return "Join the voice channel of the music bot first."
    return None


def should_leave(now, alone_since, idle_since):
    alone = alone_since is not None and now - alone_since >= ALONE_SECONDS
    idle = idle_since is not None and now - idle_since >= IDLE_SECONDS
    return alone or idle
