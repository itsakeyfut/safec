void free(void *p);
int f(void) {
    int *p = 0;
    free(p);
    return 0;
}
