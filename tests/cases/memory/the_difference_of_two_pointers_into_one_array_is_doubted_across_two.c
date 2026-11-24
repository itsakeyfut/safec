void *malloc(unsigned long long size);
void free(void *p);

int one(void) {
    int a[4];
    return (int)(&a[3] - &a[1]);
}

int apart(void) {
    int a[4];
    int b[4];
    return (int)(&a[3] - &b[1]);
}

struct pair {
    int x;
    int y;
};

int two_members(void) {
    struct pair s;
    return (int)(&s.y - &s.x);
}

struct halves {
    int a[2];
    int b[2];
};

int two_arrays_in_one(void) {
    struct halves s;
    return (int)(&s.b[0] - &s.a[0]);
}

int in_a_block(void) {
    int *p = malloc(16);
    if (!p) {
        return 0;
    }
    int d = (int)(p + 3 - p);
    free(p);
    return d;
}

int after_a_free(void) {
    char *p = malloc(8);
    if (!p) {
        return 0;
    }
    char *q = p + 4;
    free(p);
    return (int)(q - p);
}

int maybe_null(int c) {
    int a[2];
    int *p = 0;
    if (c) {
        p = a;
    }
    return (int)(a + 1 - p);
}

struct pair_of {
    int x;
    int y;
};

int members_of_an_element(void) {
    struct pair_of a[2];
    int *p = &a[0].x;
    int *q = &a[0].y;
    return (int)(q - p);
}

static struct pair_of kept[2];

int in_a_static(void) {
    return (int)(&kept[1].x - &kept[0].y);
}

union either {
    int i;
    char c[4];
};

int a_union_element(void) {
    union either a[2];
    int *p = &a[0].i;
    int *q = &a[1].i;
    return (int)(q - p);
}

struct two_chars {
    char c[2];
};

int off_the_stride(void) {
    struct two_chars a[2];
    struct two_chars *q = (struct two_chars *)((char *)a + 1);
    return (int)(q - a);
}

int rows(void) {
    int a[2][2];
    int *p = a[0];
    int *q = a[1];
    return (int)(q - p);
}

int whole_elements(void) {
    struct pair_of a[2];
    return (int)(&a[1] - &a[0]);
}

int both_off_the_stride(void) {
    struct two_chars a[4];
    struct two_chars *p = (struct two_chars *)((char *)a + 1);
    struct two_chars *q = (struct two_chars *)((char *)a + 3);
    return (int)(q - p);
}

int bytes_of_an_array(void) {
    int a[4];
    char *p = (char *)a;
    char *q = (char *)(a + 2);
    return (int)(q - p);
}

int off_by_an_unknown(int n) {
    struct two_chars a[4];
    struct two_chars *p = (struct two_chars *)((char *)a + n);
    return (int)(p - a);
}

int moved_by_n(int n) {
    int a[4];
    int *p = a + n;
    return (int)(p - a);
}

static int kept_ints[4];

int in_a_static_array(void) {
    return (int)(&kept_ints[3] - &kept_ints[0]);
}

int main(void) {
    return one() + apart() + two_members() + two_arrays_in_one() + in_a_block() + after_a_free()
        + maybe_null(1) + members_of_an_element() + in_a_static() + a_union_element()
        + off_the_stride() + rows() + whole_elements() + both_off_the_stride()
        + bytes_of_an_array() + off_by_an_unknown(1) + moved_by_n(2) + in_a_static_array();
}
