void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
    for (; *p; ) { return 1; }
    return 0;
}
