void *memcpy(void *d, void *s, int n);
void g(int *(**out)[1]) {
    int *m;
    memcpy(*out, &m, 8);
}
