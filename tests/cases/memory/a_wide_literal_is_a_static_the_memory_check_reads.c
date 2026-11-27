int past(void) {
    const unsigned short *p = L"ab";
    return p[3];
}

int written(void) {
    unsigned short *p = (unsigned short *)L"ab";
    p[0] = 1;
    return 0;
}

int local(void) {
    unsigned short w[4] = L"ab";
    return w[3] + w[4];
}
