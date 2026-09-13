void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    while (*p) { free(p); }
    return 0;
}
