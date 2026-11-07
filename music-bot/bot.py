import asyncio
import io
import logging
import os
import sys
import time

import discord
from discord import app_commands
from player import Player
from rules import NICKNAME, ROLE, activity, control_refusal, play_refusal, should_leave
from status import Status, serve, summary

from jellyfin import Jellyfin

JELLYFIN_URL = "http://jellyfin:8096"
STATUS_PORT = 8000
TICK_SECONDS = 15
QUEUE_SHOWN = 10
KIND_NAMES = {"MusicArtist": "artist", "MusicAlbum": "album", "Playlist": "playlist", "Audio": "track"}

log = logging.getLogger("music-bot")


def required(name):
    value = os.environ.get(name, "").strip()
    if not value:
        sys.exit(f"{name} is empty in .env")
    return value


def is_dj(member):
    return isinstance(member, discord.Member) and any(role.name == ROLE for role in member.roles)


def voice_channel(member):
    voice = getattr(member, "voice", None)
    return voice.channel if voice and voice.channel else None


def channel_id(channel):
    return channel.id if channel else None


def cut(text, size=100):
    return text if len(text) <= size else text[: size - 1] + "…"


class MusicBot(discord.Client):
    def __init__(self, guild_id, jellyfin, status):
        super().__init__(intents=discord.Intents.default())
        self.guild = discord.Object(id=guild_id)
        self.tree = app_commands.CommandTree(self)
        self.jellyfin = jellyfin
        self.report = status
        self.player = Player()
        self.voice = None
        self.volume = 0.5
        self.text_channel = None
        self.now_playing = None
        self.alone_since = None
        self.idle_since = None
        self.lock = asyncio.Lock()
        register(self)

    async def setup_hook(self):
        self.add_view(Controls(self))
        await self.tree.sync(guild=self.guild)
        self.loop.create_task(self.watch())

    def connected(self):
        return self.voice is not None and self.voice.is_connected()

    def channel(self):
        return self.voice.channel if self.connected() else None

    def playing(self):
        return self.connected() and self.voice.is_playing()

    def paused(self):
        return self.connected() and self.voice.is_paused()

    def busy(self):
        return self.playing() or self.paused() or bool(self.player.queue) or self.player.shuffle

    def play_refusal(self, member):
        return play_refusal(is_dj(member), channel_id(voice_channel(member)), channel_id(self.channel()), self.busy())

    def control_refusal(self, member):
        return control_refusal(is_dj(member), channel_id(voice_channel(member)), channel_id(self.channel()))

    async def join(self, channel):
        if self.connected():
            if self.voice.channel.id != channel.id:
                await self.voice.move_to(channel)
            return
        self.voice = await channel.connect(self_deaf=True)

    async def begin(self, interaction):
        self.text_channel = interaction.channel
        await self.join(voice_channel(interaction.user))
        await self.play_next()

    async def play_next(self):
        async with self.lock:
            if not self.connected() or self.voice.is_playing() or self.voice.is_paused():
                return
            if self.player.needs_library():
                try:
                    self.player.refill(await asyncio.to_thread(self.jellyfin.library))
                except Exception:
                    log.exception("Jellyfin did not list the Library")
                    self.player.shuffle = False
            track = self.player.advance()
            if track is None:
                await self.show(None)
                return
            source = discord.PCMVolumeTransformer(discord.FFmpegPCMAudio(track.path, options="-vn"), self.volume)
            self.voice.play(source, after=self.finished)
            await self.show(track)

    def finished(self, error):
        if error:
            log.error("Playback failed: %s", error)
        asyncio.run_coroutine_threadsafe(self.play_next(), self.loop)

    async def rename(self):
        guild = self.get_guild(self.guild.id)
        if guild is None or guild.me.nick == NICKNAME:
            return
        try:
            await guild.me.edit(nick=NICKNAME)
        except discord.HTTPException:
            log.exception("Could not rename the music bot")

    async def listen(self, track):
        name = activity(track)
        listening = discord.Activity(type=discord.ActivityType.listening, name=name) if name else None
        try:
            await self.change_presence(activity=listening)
        except Exception:
            log.exception("Could not set the status of the music bot")

    async def cover(self, track):
        try:
            return await asyncio.to_thread(self.jellyfin.cover, track)
        except Exception:
            log.exception("Jellyfin did not give the cover of %s", track.id)
            return None

    async def show(self, track):
        await self.listen(track)
        if self.now_playing:
            try:
                await self.now_playing.delete()
            except discord.HTTPException:
                pass
            self.now_playing = None
        if track and self.text_channel:
            embed = discord.Embed(title=track.title, description=track.artist or None)
            if track.album:
                embed.set_footer(text=track.album)
            source = "Queue" if self.player.queue or not self.player.shuffle else "Shuffle"
            embed.set_author(name=f"Now playing, from the {source}")
            image = await self.cover(track)
            files = []
            if image:
                files.append(discord.File(io.BytesIO(image), filename="cover.jpg"))
                embed.set_thumbnail(url="attachment://cover.jpg")
            try:
                self.now_playing = await self.text_channel.send(embed=embed, files=files, view=Controls(self))
            except discord.HTTPException:
                log.exception("Could not post the track now playing")

    async def halt(self):
        self.player.stop()
        if self.voice:
            self.voice.stop()
            if self.voice.is_connected():
                await self.voice.disconnect()
        self.voice = None
        self.alone_since = None
        self.idle_since = None
        await self.show(None)
        await self.rename()

    async def toggle_pause(self):
        if self.voice.is_paused():
            self.voice.resume()
            return "Resumed."
        if self.voice.is_playing():
            self.voice.pause()
            return "Paused."
        return "Nothing is playing."

    def skip(self):
        if self.voice.is_playing() or self.voice.is_paused():
            self.voice.stop()
            return "Skipped."
        return "Nothing is playing."

    def snapshot(self):
        channel = self.channel()
        return summary(self.player, self.playing(), self.paused(), channel.name if channel else None)

    async def watch(self):
        await self.wait_until_ready()
        while not self.is_closed():
            try:
                await self.tick()
            except Exception:
                log.exception("The watch loop failed")
            await asyncio.sleep(TICK_SECONDS)

    async def tick(self):
        now = time.monotonic()
        if self.voice and not self.voice.is_connected():
            await self.halt()
        if self.connected():
            listeners = [member for member in self.voice.channel.members if not member.bot]
            self.alone_since = None if listeners else self.alone_since or now
            self.idle_since = None if self.voice.is_playing() else self.idle_since or now
            if should_leave(now, self.alone_since, self.idle_since):
                await self.halt()
        self.report.update(self.is_ready() and not self.is_closed(), self.snapshot())


class Controls(discord.ui.View):
    def __init__(self, bot):
        super().__init__(timeout=None)
        self.bot = bot

    async def interaction_check(self, interaction):
        refusal = self.bot.control_refusal(interaction.user)
        if refusal:
            await interaction.response.send_message(refusal, ephemeral=True)
            return False
        return True

    @discord.ui.button(label="Pause", emoji="⏯️", style=discord.ButtonStyle.secondary, custom_id="music-bot:pause")
    async def pause(self, interaction, button):
        await interaction.response.send_message(await self.bot.toggle_pause(), ephemeral=True)

    @discord.ui.button(label="Skip", emoji="⏭️", style=discord.ButtonStyle.primary, custom_id="music-bot:skip")
    async def skip(self, interaction, button):
        await interaction.response.send_message(self.bot.skip(), ephemeral=True)

    @discord.ui.button(label="Stop", emoji="⏹️", style=discord.ButtonStyle.danger, custom_id="music-bot:stop")
    async def stop_playing(self, interaction, button):
        await interaction.response.send_message("Stopped.", ephemeral=True)
        await self.bot.halt()


class Results(discord.ui.View):
    def __init__(self, bot, author, hits):
        super().__init__(timeout=120)
        self.bot = bot
        self.author = author
        self.hits = {f"{hit.kind}:{hit.id}": hit for hit in hits}
        menu = discord.ui.Select(
            placeholder="Pick what to play",
            options=[
                discord.SelectOption(label=cut(hit.name or "Untitled"), description=cut(hit.detail), value=value)
                for value, hit in self.hits.items()
            ],
        )
        menu.callback = self.picked
        self.add_item(menu)

    async def interaction_check(self, interaction):
        return interaction.user.id == self.author

    async def picked(self, interaction):
        hit = self.hits[interaction.data["values"][0]]
        refusal = self.bot.play_refusal(interaction.user)
        if refusal:
            await interaction.response.edit_message(content=refusal, view=None)
            return
        await interaction.response.edit_message(content=f"Loading {cut(hit.name, 80)}…", view=None)
        try:
            tracks = await asyncio.to_thread(self.bot.jellyfin.tracks, hit)
        except Exception:
            log.exception("Jellyfin did not list the tracks of %s", hit.id)
            await interaction.edit_original_response(content="Jellyfin does not answer, try again.")
            return
        if not tracks:
            await interaction.edit_original_response(content=f"{hit.name} has no track to play.")
            return
        count = self.bot.player.add(tracks, shuffled=hit.kind == "MusicArtist")
        kind = KIND_NAMES[hit.kind]
        await interaction.edit_original_response(content=f"Added the {kind} {hit.name}, {count} track(s).")
        await interaction.channel.send(f"{interaction.user.mention} added the {kind} **{hit.name}** to the Queue.")
        await self.bot.begin(interaction)


def register(bot):
    tree, guild = bot.tree, bot.guild

    @tree.command(name="play", description="Play the whole Library in Shuffle, or search the Library", guild=guild)
    @app_commands.describe(search="Artist, album, playlist or track to search, empty for the Shuffle")
    async def play(interaction: discord.Interaction, search: str | None = None):
        refusal = bot.play_refusal(interaction.user)
        if refusal:
            await interaction.response.send_message(refusal, ephemeral=True)
            return
        if not search or not search.strip():
            started = bot.player.start_shuffle()
            await interaction.response.send_message(
                "Shuffle of the whole Library is on." if started else "Shuffle is already on."
            )
            await bot.begin(interaction)
            return
        await interaction.response.defer(ephemeral=True, thinking=True)
        try:
            hits, hidden = await asyncio.to_thread(bot.jellyfin.search, search.strip())
        except Exception:
            log.exception("Jellyfin did not search %r", search)
            await interaction.followup.send("Jellyfin does not answer, try again.", ephemeral=True)
            return
        if not hits:
            await interaction.followup.send(f"Nothing in the Library matches {search}.", ephemeral=True)
            return
        note = f"\n{hidden} more result(s), refine the search to see them." if hidden else ""
        await interaction.followup.send(
            f"Results for **{search}**:{note}", view=Results(bot, interaction.user.id, hits), ephemeral=True
        )

    async def control(interaction, action):
        refusal = bot.control_refusal(interaction.user)
        if refusal:
            await interaction.response.send_message(refusal, ephemeral=True)
            return
        await interaction.response.send_message(await action())

    @tree.command(name="skip", description="Skip the track playing", guild=guild)
    async def skip(interaction: discord.Interaction):
        async def action():
            return bot.skip()

        await control(interaction, action)

    @tree.command(name="pause", description="Pause or resume the track playing", guild=guild)
    async def pause(interaction: discord.Interaction):
        await control(interaction, bot.toggle_pause)

    @tree.command(name="stop", description="Empty the Queue, end the Shuffle and leave the voice channel", guild=guild)
    async def stop(interaction: discord.Interaction):
        async def action():
            await bot.halt()
            return "Stopped."

        await control(interaction, action)

    @tree.command(name="volume", description="Set the volume of the music bot", guild=guild)
    @app_commands.describe(percent="From 0 to 100")
    async def volume(interaction: discord.Interaction, percent: app_commands.Range[int, 0, 100]):
        async def action():
            bot.volume = percent / 100
            if isinstance(bot.voice.source, discord.PCMVolumeTransformer):
                bot.voice.source.volume = bot.volume
            return f"Volume at {percent}%."

        await control(interaction, action)

    @tree.command(name="queue", description="Show the track playing and the Queue", guild=guild)
    async def queue(interaction: discord.Interaction):
        if not is_dj(interaction.user):
            await interaction.response.send_message(f"Only a member with the {ROLE} role can command the music bot.",
                                                    ephemeral=True)
            return
        player = bot.player
        lines = []
        if player.current and (bot.playing() or bot.paused()):
            lines.append(f"Now: **{player.current.label}**")
        lines += [f"{number}. {track.label}" for number, track in enumerate(list(player.queue)[:QUEUE_SHOWN], 1)]
        if len(player.queue) > QUEUE_SHOWN:
            lines.append(f"And {len(player.queue) - QUEUE_SHOWN} more.")
        lines.append(f"Shuffle: {'on' if player.shuffle else 'off'}.")
        await interaction.response.send_message(cut("\n".join(lines), 2000), ephemeral=True)


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")
    token = required("DISCORD_TOKEN")
    guild = required("DISCORD_GUILD_ID")
    if not guild.isdigit():
        sys.exit(f"DISCORD_GUILD_ID must be the number of the Discord server, got {guild!r}")
    status = Status()
    serve(status, "0.0.0.0", STATUS_PORT)
    bot = MusicBot(int(guild), Jellyfin(JELLYFIN_URL, required("MUSIC_BOT_JELLYFIN_API_KEY")), status)
    bot.run(token, log_handler=None)


if __name__ == "__main__":
    main()
