void free(void *p);
#include "an_included_free_is_listed_where_its_include_is.h"

void k(int *q) {
    free(q);
    free(q);
}
