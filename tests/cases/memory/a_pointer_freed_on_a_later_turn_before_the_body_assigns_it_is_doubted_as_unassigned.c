void *malloc(int n); void free(void *p);
int *find(int k);
int f(void) {
    int *p;
    for (int i = 0; i < 4; i++) { if (i > 0) free(p); p = find(i); }
    return 0;
}
