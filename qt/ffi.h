#pragma once
// C ABI surface used by Catalog (mirrors catalog-ffi; JSON in/out).
// Blocking calls with the tokio runtime inside — invoke off the UI thread.
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct CatalogBytes {
    char *ptr;
    size_t len;
} CatalogBytes;

char *catalog_version(void);
char *catalog_abi(void);
void catalog_string_free(char *s);
void catalog_bytes_free(CatalogBytes b);

char *catalog_open_count(const char *config_json);
char *catalog_fetch_books(const char *config_json, const char *query,
                          bool sort_descending, bool sort_by_author, bool sort_by_date);
char *catalog_browse(const char *config_json, const char *mode,
                     bool sort_descending, bool sort_by_author);
char *catalog_search(const char *config_json, const char *params_json);
char *catalog_detail(const char *config_json, long long book_id);
CatalogBytes catalog_cover(const char *config_json, const char *book_path,
                           unsigned max_w, unsigned max_h);

char *catalog_kobo_predict(const char *title, const char *author_sort,
                           const char *authors_natural);
char *catalog_kobo_match(const char *candidates_json, const char *title,
                         const char *author);
char *catalog_kobo_open(const char *config_json);
char *catalog_kobo_sync(const char *config_json);
char *catalog_kobo_ssh(const char *config_json);
char *catalog_kobo_handoff_check(const char *kobo_ip);
char *catalog_kobo_handoff_ensure(const char *kobo_ip);

#ifdef __cplusplus
}
#endif
