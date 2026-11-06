void *memcpy(void *d, void *s, int n);
void f(int **pp) {
    int k = 1;
    memcpy(*pp, &k, 4);
}
