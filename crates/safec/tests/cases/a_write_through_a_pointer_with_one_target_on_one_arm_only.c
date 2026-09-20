void *malloc(int n);
void free(void *p);
int **other(void);
int f(int c) {
    int *p = malloc(4);
    int *q = malloc(8);
    int **pp = other();
    if (c) { pp = &p; }
    *pp = q;
    free(p);
    *q = 1;
    return 0;
}
