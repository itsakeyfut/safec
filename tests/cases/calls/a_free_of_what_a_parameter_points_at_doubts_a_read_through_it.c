void free(void *p);

int f(int **pp) {
    if (pp == 0) {
        return 0;
    }
    free(*pp);
    return **pp;
}
