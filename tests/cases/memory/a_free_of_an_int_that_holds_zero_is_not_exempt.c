void free(void *p);
int f(void) {
    int x = 0;
    free(x);
    return 0;
}
