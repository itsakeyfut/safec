void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
#ifdef __x86_64__
    /* nothing more to release */
#else
    free(p);
#endif
    return 0;
}
