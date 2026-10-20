void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
#ifdef U
#define X \
#else
    p = malloc(4);
#endif
    free(p);
    return 0;
}
