void *malloc(int n);
void free(void *p);
int f(int c, int *p) {
    if (c) { free(p); }
    *p = 1;
    return 0;
}
