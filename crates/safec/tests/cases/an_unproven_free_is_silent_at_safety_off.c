void *malloc(int n);
void free(void *p);
int f(int c, int *p) {
    if (c) { free(p); }
    free(p);
    return 0;
}
