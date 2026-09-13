void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    int **pp;
    if (c) { pp = &p; }
    p = malloc(8);
    free(p);
    return 0;
}
