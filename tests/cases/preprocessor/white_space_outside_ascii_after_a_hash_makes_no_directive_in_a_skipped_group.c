void *malloc(unsigned long long);
void free(void *);

int chosen(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    free(p);
#if 0
# else
    return 0;
#endif
    free(p);
    return 0;
}

int counted(void) {
    int *p = malloc(sizeof *p);
    if (!p) return 1;
    *p = 1;
    free(p);
#if 0
# ifdef X
#else
    return *p;
#endif
#if 0
# endif
#endif
    return 0;
}
