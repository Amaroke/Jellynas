const env = process.env;

const indexerIds = (env.CROSS_SEED_INDEXER_IDS || "")
	.split(",")
	.map((id) => id.trim())
	.filter(Boolean);

module.exports = {
	apiKey: env.CROSS_SEED_API_KEY,
	torznab: indexerIds.map((id) => `http://prowlarr:9696/${id}/api?apikey=${env.PROWLARR_API_KEY}`),
	sonarr: [`http://sonarr:8989/?apikey=${env.SONARR_API_KEY}`],
	radarr: [`http://radarr:7878/?apikey=${env.RADARR_API_KEY}`],
	host: "0.0.0.0",
	port: 2468,
	torrentClients: [
		`qbittorrent:http://${encodeURIComponent(env.QBITTORRENT_USER)}:${encodeURIComponent(env.QBITTORRENT_PASSWORD)}@gluetun:8080`,
	],
	useClientTorrents: true,
	linkCategory: "farm",
	linkDirs: ["/data/torrents/cross-seed"],
	linkType: "hardlink",
	flatLinking: false,
	matchMode: "flexible",
	skipRecheck: true,
	includeSingleEpisodes: false,
	includeNonVideos: false,
	seasonFromEpisodes: 1,
	excludeOlder: "2 weeks",
	excludeRecentSearch: "3 days",
	action: "inject",
	duplicateCategories: false,
	rssCadence: "30 minutes",
	searchCadence: "1 day",
	delay: 30,
	blockList: [],
};
