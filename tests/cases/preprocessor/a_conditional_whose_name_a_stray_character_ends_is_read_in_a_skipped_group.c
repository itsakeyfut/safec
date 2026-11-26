void *malloc(unsigned long long);
void free(void *);

int at(void) {
    char *b = malloc(1);
    if (!b) return 0;
#if 0
#else@
    b[1] = 0;
#endif
    free(b);
    return 0;
}

int space(void) {
    char *b = malloc(1);
    if (!b) return 0;
#if 0
#else　
    b[1] = 0;
#endif
    free(b);
    return 0;
}
