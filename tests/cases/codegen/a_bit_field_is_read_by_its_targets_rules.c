struct Packed {
    char c;
    int : 17;
    char d;
    char plain : 3;
};

int size(void) {
    return sizeof(struct Packed);
}

int read(struct Packed *p) {
    p->d = 1;
    return p->plain;
}
