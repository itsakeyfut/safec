struct S {
    int a;
};

void *memcpy(void *into, const void *from, unsigned long long size) {
    unsigned char *to = into;
    const unsigned char *bytes = from;
    for (unsigned long long i = 0; i < size; i++) {
        to[i] = bytes[i];
    }
    return into;
}

int copied(struct S a) {
    struct S b;
    b = a;
    return b.a;
}
