void free(void *p);
void get(int **pp);
int f(void) {
    int *u;
    get(&u);
    free(u);
    return 0;
}
