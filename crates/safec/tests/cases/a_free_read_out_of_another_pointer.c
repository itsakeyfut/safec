void free(void *p);
int f(int **pp) {
    free(*pp);
    return 0;
}
