void *malloc(unsigned long long);
void free(void *);

#define GUARD
int ifdef(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    *p = 1;
    free(p);
#ifdef GUARD\N{LATIN SMALL LETTER E WITH ACUTE}
#else
    free(p);
#endif
    return 0;
}

int skipped(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    *p = 1;
    free(p);
#if 0
#else\N{NO-BREAK SPACE}
    return *p;
#endif
    return 0;
}

#pragma once\N{LATIN SMALL LETTER E WITH ACUTE}
