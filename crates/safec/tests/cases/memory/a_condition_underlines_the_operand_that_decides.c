void *malloc(int n);
void free(void *p);

int if_comma(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    if (c, *p) {
        return 1;
    }
    return 0;
}

int if_chain(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    if (c, c, *p) {
        return 1;
    }
    return 0;
}

int if_nested(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    if (c, (c, *p)) {
        return 1;
    }
    return 0;
}

int if_left(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    if (*p, c) {
        return 1;
    }
    return 0;
}

int while_comma(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    while (c, *p) {
        return 1;
    }
    return 0;
}

int for_comma(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    for (; c, *p;) {
        return 1;
    }
    return 0;
}

int conditional_comma(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    return (c, *p) ? 1 : 2;
    return 0;
}

int and_left(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    return *p && c;
    return 0;
}

int or_right(int c) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    return c || *p;
    return 0;
}
