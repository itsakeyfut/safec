void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    free(p);
    if (c, *p) { return 1; }
    return 0;
}
