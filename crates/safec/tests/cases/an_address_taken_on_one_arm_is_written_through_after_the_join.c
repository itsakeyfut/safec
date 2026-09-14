void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    int *q = malloc(8);
    int **pp = 0;
    if (c) { pp = &p; }
    *pp = q;
    free(p);
    *q = 1;
    return 0;
}
