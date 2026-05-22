package expo.modules.rustbridge

import android.content.Context
import android.net.Uri
import androidx.documentfile.provider.DocumentFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.text.Normalizer
import java.util.Locale

object ExistingDownloadScanner {
  private const val PREFS_NAME = "app_settings"
  private const val PREF_DOWNLOAD_DIRECTORY = "download_directory"
  private const val PAGE_SIZE = 500
  private const val MIN_MATCH_SCORE = 700
  private const val MIN_SCORE_MARGIN = 40

  data class ScanResult(
    val filesScanned: Int,
    val booksMatched: Int,
    val booksLinked: Int,
    val booksAlreadyLinked: Int,
    val filesUnmatched: Int,
    val ambiguousMatches: Int,
    val errors: List<String>
  ) {
    fun toMap(): Map<String, Any?> = mapOf(
      "files_scanned" to filesScanned,
      "books_matched" to booksMatched,
      "books_linked" to booksLinked,
      "books_already_linked" to booksAlreadyLinked,
      "files_unmatched" to filesUnmatched,
      "ambiguous_matches" to ambiguousMatches,
      "errors" to errors
    )
  }

  private data class BookCandidate(
    val asin: String,
    val title: String,
    val authors: List<String>,
    val seriesName: String?,
    val filePath: String?
  ) {
    val normalizedTitle: String = normalize(title)
    val compactTitle: String = compact(title)
    val titleTokens: Set<String> = tokens(title)
    val compactAuthors: List<String> = authors.map(::compact).filter { it.length >= 5 }
    val compactSeries: String = seriesName?.let(::compact).orEmpty()
  }

  private data class AudioCandidate(
    val name: String,
    val uri: String,
    val pathParts: List<String>
  ) {
    val stem: String = name.substringBeforeLast('.', name)
    val searchText: String = normalize((pathParts + name + Uri.decode(uri)).joinToString(" "))
    val compactSearchText: String = compact((pathParts + name + Uri.decode(uri)).joinToString(" "))
    val rawSearchText: String = ((pathParts + name + Uri.decode(uri)).joinToString(" ")).uppercase(Locale.US)
  }

  private data class MatchCandidate(
    val file: AudioCandidate,
    val book: BookCandidate,
    val score: Int
  )

  fun saveDownloadDirectory(context: Context, directory: String) {
    context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
      .edit()
      .putString(PREF_DOWNLOAD_DIRECTORY, directory)
      .apply()
  }

  fun getSavedDownloadDirectory(context: Context): String? {
    return context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
      .getString(PREF_DOWNLOAD_DIRECTORY, null)
  }

  fun scan(context: Context, dbPath: String, downloadDirectory: String): ScanResult {
    val errors = mutableListOf<String>()
    val books = loadBooks(dbPath, errors)

    if (books.isEmpty()) {
      return ScanResult(0, 0, 0, 0, 0, 0, errors)
    }

    val files = scanAudioFiles(context, downloadDirectory, errors)
    val matchesByAsin = mutableMapOf<String, MatchCandidate>()
    var filesUnmatched = 0
    var ambiguousMatches = 0

    for (file in files) {
      val match = findBestMatch(file, books)
      if (match == null) {
        filesUnmatched += 1
        continue
      }

      if (match.score < 1000 && isAmbiguous(file, books, match)) {
        ambiguousMatches += 1
        continue
      }

      val existing = matchesByAsin[match.book.asin]
      if (existing == null || match.score > existing.score) {
        matchesByAsin[match.book.asin] = match
      }
    }

    var linked = 0
    var alreadyLinked = 0

    for (match in matchesByAsin.values) {
      if (match.book.filePath == match.file.uri) {
        alreadyLinked += 1
        continue
      }

      if (linkBookToFile(dbPath, match.book, match.file, errors)) {
        linked += 1
      }
    }

    return ScanResult(
      filesScanned = files.size,
      booksMatched = matchesByAsin.size,
      booksLinked = linked,
      booksAlreadyLinked = alreadyLinked,
      filesUnmatched = filesUnmatched,
      ambiguousMatches = ambiguousMatches,
      errors = errors
    )
  }

  private fun loadBooks(dbPath: String, errors: MutableList<String>): List<BookCandidate> {
    val books = mutableListOf<BookCandidate>()
    var offset = 0
    var totalCount = Int.MAX_VALUE

    while (offset < totalCount) {
      val params = JSONObject().apply {
        put("db_path", dbPath)
        put("offset", offset)
        put("limit", PAGE_SIZE)
        put("sort_field", "title")
        put("sort_direction", "asc")
      }

      val response = JSONObject(ExpoRustBridgeModule.nativeGetBooksWithFilters(params.toString()))
      if (!response.optBoolean("success")) {
        errors.add(response.optString("error", "Failed to load books for download scan"))
        break
      }

      val data = response.optJSONObject("data") ?: break
      totalCount = data.optInt("total_count", offset)
      val pageBooks = data.optJSONArray("books") ?: JSONArray()
      if (pageBooks.length() == 0) break

      for (i in 0 until pageBooks.length()) {
        val book = pageBooks.optJSONObject(i) ?: continue
        val asin = book.optString("audible_product_id")
        val title = book.optString("title")
        if (asin.isBlank() || title.isBlank()) continue

        books.add(
          BookCandidate(
            asin = asin,
            title = title,
            authors = jsonStringArray(book.optJSONArray("authors")),
            seriesName = book.optStringOrNull("series_name"),
            filePath = book.optStringOrNull("file_path")
          )
        )
      }

      offset += pageBooks.length()
    }

    return books
  }

  private fun scanAudioFiles(
    context: Context,
    downloadDirectory: String,
    errors: MutableList<String>
  ): List<AudioCandidate> {
    return if (downloadDirectory.startsWith("content://")) {
      val rootUri = Uri.parse(downloadDirectory)
      val root = DocumentFile.fromTreeUri(context, rootUri)
      if (root == null || !root.isDirectory) {
        errors.add("Download directory is not accessible")
        emptyList()
      } else {
        val files = mutableListOf<AudioCandidate>()
        scanDocumentDirectory(root, listOf(root.name ?: ""), files)
        files
      }
    } else {
      val root = File(downloadDirectory.removePrefix("file://"))
      if (!root.exists() || !root.isDirectory) {
        errors.add("Download directory is not accessible")
        emptyList()
      } else {
        root.walkTopDown()
          .filter { it.isFile && isAudioFile(it.name, null) }
          .map { file ->
            val relative = runCatching {
              root.toPath().relativize(file.toPath()).map { it.toString() }
            }.getOrDefault(listOf(file.name))
            AudioCandidate(file.name, file.absolutePath, relative)
          }
          .toList()
      }
    }
  }

  private fun scanDocumentDirectory(
    directory: DocumentFile,
    pathParts: List<String>,
    files: MutableList<AudioCandidate>
  ) {
    directory.listFiles().forEach { child ->
      val name = child.name ?: return@forEach
      if (child.isDirectory) {
        scanDocumentDirectory(child, pathParts + name, files)
      } else if (child.isFile && isAudioFile(name, child.type)) {
        files.add(AudioCandidate(name, child.uri.toString(), pathParts + name))
      }
    }
  }

  private fun findBestMatch(file: AudioCandidate, books: List<BookCandidate>): MatchCandidate? {
    return books
      .mapNotNull { book ->
        val score = score(file, book)
        if (score >= MIN_MATCH_SCORE) MatchCandidate(file, book, score) else null
      }
      .maxByOrNull { it.score }
  }

  private fun isAmbiguous(
    file: AudioCandidate,
    books: List<BookCandidate>,
    best: MatchCandidate
  ): Boolean {
    val secondBest = books
      .asSequence()
      .filter { it.asin != best.book.asin }
      .map { score(file, it) }
      .filter { it >= MIN_MATCH_SCORE }
      .maxOrNull()

    return secondBest != null && best.score - secondBest < MIN_SCORE_MARGIN
  }

  private fun score(file: AudioCandidate, book: BookCandidate): Int {
    if (book.asin.isNotBlank() && file.rawSearchText.contains(book.asin.uppercase(Locale.US))) {
      return 1000
    }

    var score = 0
    val normalizedStem = normalize(file.stem)

    if (book.normalizedTitle.isNotBlank() && normalizedStem == book.normalizedTitle) {
      score = maxOf(score, 850)
    }

    if (book.normalizedTitle.length >= 8 && file.searchText.contains(book.normalizedTitle)) {
      score = maxOf(score, 780)
    }

    if (book.compactTitle.length >= 8 && file.compactSearchText.contains(book.compactTitle)) {
      score = maxOf(score, 760)
    }

    val coverage = tokenCoverage(book.titleTokens, file.searchText)
    if (coverage >= 0.9 && book.titleTokens.size >= 2) {
      score = maxOf(score, 680)
    } else if (coverage >= 0.75 && book.titleTokens.size >= 3) {
      score = maxOf(score, 620)
    }

    if (score > 0 && book.compactAuthors.any { file.compactSearchText.contains(it) }) {
      score += 150
    }

    if (score > 0 && book.compactSeries.length >= 5 && file.compactSearchText.contains(book.compactSeries)) {
      score += 80
    }

    return score
  }

  private fun tokenCoverage(tokens: Set<String>, text: String): Double {
    if (tokens.isEmpty()) return 0.0
    val matched = tokens.count { token -> text.contains(token) }
    return matched.toDouble() / tokens.size.toDouble()
  }

  private fun linkBookToFile(
    dbPath: String,
    book: BookCandidate,
    file: AudioCandidate,
    errors: MutableList<String>
  ): Boolean {
    val params = JSONObject().apply {
      put("db_path", dbPath)
      put("asin", book.asin)
      put("title", book.title)
      put("file_path", file.uri)
    }

    val response = JSONObject(ExpoRustBridgeModule.nativeSetBookFilePath(params.toString()))
    if (!response.optBoolean("success")) {
      errors.add("${book.title}: ${response.optString("error", "Failed to link file")}")
      return false
    }

    return true
  }

  private fun jsonStringArray(array: JSONArray?): List<String> {
    if (array == null) return emptyList()
    return (0 until array.length()).mapNotNull { index ->
      array.optString(index).takeIf { it.isNotBlank() }
    }
  }

  private fun JSONObject.optStringOrNull(name: String): String? {
    if (!has(name) || isNull(name)) return null
    return optString(name).takeIf { it.isNotBlank() }
  }

  private fun tokens(value: String): Set<String> {
    return normalize(value)
      .split(' ')
      .map { it.trim() }
      .filter { it.length >= 3 && it !in STOP_WORDS }
      .toSet()
  }

  private fun normalize(value: String): String {
    val ascii = Normalizer.normalize(value, Normalizer.Form.NFD)
      .replace("\\p{Mn}+".toRegex(), "")

    return ascii
      .lowercase(Locale.US)
      .replace("[^a-z0-9]+".toRegex(), " ")
      .trim()
      .replace("\\s+".toRegex(), " ")
  }

  private fun compact(value: String): String {
    return normalize(value).replace(" ", "")
  }

  private fun isAudioFile(displayName: String, mimeType: String?): Boolean {
    if (mimeType?.startsWith("audio/") == true) {
      return true
    }

    val lowerName = displayName.lowercase(Locale.US)
    return lowerName.endsWith(".m4b") ||
      lowerName.endsWith(".m4a") ||
      lowerName.endsWith(".mp4") ||
      lowerName.endsWith(".mp3") ||
      lowerName.endsWith(".aac") ||
      lowerName.endsWith(".flac") ||
      lowerName.endsWith(".ogg") ||
      lowerName.endsWith(".opus") ||
      lowerName.endsWith(".wav")
  }

  private val STOP_WORDS = setOf(
    "the", "and", "for", "with", "from", "into", "onto", "book", "part", "vol",
    "volume", "edition", "unabridged", "audiobook"
  )
}
