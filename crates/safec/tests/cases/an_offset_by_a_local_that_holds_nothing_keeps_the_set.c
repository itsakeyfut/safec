void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (c) { p = malloc(8); }
    int n = 1;
    free(p);
    p = p + n;
    free(p);
    return 0;
}
