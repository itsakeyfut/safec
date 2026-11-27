union Short {
    char c;
    int *p;
};

int fits(void) {
    int *bound[1024] = {0};
    return bound[0] == 0;
}

int past(void) {
    int *many[1025] = {0};
    return many[0] == 0;
}

int short_first(void) {
    union Short s = {1};
    return s.c;
}
