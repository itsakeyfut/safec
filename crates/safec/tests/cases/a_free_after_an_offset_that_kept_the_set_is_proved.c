void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    if (c) { p = malloc(8); }
    free(p);
    p = p + 1;
    free(p);
    return 0;
}
