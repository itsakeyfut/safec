void *memcpy(void *d, const void *s, unsigned long long n);
void set();
static const int k = 1;
long long returned(void) {
    const int *q = &k;
    long long v;
    memcpy(&v, &q, sizeof v);
    return v;
}
int main(void) {
    const int *q = &k;
    long long v;
    memcpy(&v, &q, sizeof v);
    set(v);
    return 0;
}
