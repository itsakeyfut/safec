void *memcpy(void *d, void *s, int n);
void h(int *(**out)[1]) {
    int v = 0;
    memcpy(*out, &v, 4);
}
