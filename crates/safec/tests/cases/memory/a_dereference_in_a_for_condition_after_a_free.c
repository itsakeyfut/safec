void *malloc(int n);
void free(void *p);
int f(int i) {
    int *p = malloc(8);
    free(p);
    for (i = 0; *p; i = i + 1) { return 1; }
    return 0;
}
