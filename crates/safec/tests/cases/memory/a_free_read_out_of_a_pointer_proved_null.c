void free(void *p);
int f(void) {
    int **pp = 0;
    free(*pp);
    return 0;
}
