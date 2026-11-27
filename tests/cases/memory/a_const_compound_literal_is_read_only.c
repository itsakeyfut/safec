void *memchr(const void *s, int c, unsigned long long n);

union U {
    const int *c;
    int *p;
};

int main(void) {
    union U u;
    u.c = (const int[]){1, 2};
    int *q = u.p;
    if (q) {
        *q = 3;
    }
    return 0;
}

int through_a_library_call(void) {
    char *q = memchr((const char[]){1, 2}, 2, 2);
    if (q) {
        *q = 3;
    }
    return 0;
}
