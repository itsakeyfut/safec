static int calls;

static int counted(void) {
    calls++;
    return 4;
}

static int stepped;

static int step(void) {
    stepped++;
    return 2;
}

int main(void) {
    int wrong = 0;

    // The length is worked out where the typedef is (C17 6.7.8 p3), not
    // where an object of the type is declared.
    int n = 3;
    typedef char Later[n];
    n = 7;
    Later later;
    wrong += sizeof later == 3 ? 0 : 1;

    // Once, however many objects are declared with it.
    int m = 3;
    typedef char Once[m++];
    Once first;
    Once second;
    wrong += m == 4 && sizeof first == 3 && sizeof second == 3 ? 0 : 1;

    // Even with no object at all.
    typedef int Unused[counted()];
    wrong += calls == 1 ? 0 : 1;

    // `sizeof` of the type itself, when it runs.
    int k = 5;
    typedef long long Wide[k];
    wrong += sizeof(Wide) == 5 * sizeof(long long) ? 0 : 1;

    // Afresh on each pass of a loop.
    int total = 0;
    for (int i = 1; i <= 3; i++) {
        typedef char Pass[i];
        Pass a;
        total += (int)sizeof a;
    }
    wrong += total == 6 ? 0 : 1;

    // The `s` in scope where the typedef is, not a later one that hides it.
    int s = 3;
    typedef char Scoped[s];
    {
        int s = 9;
        Scoped a;
        wrong += sizeof a == 3 && s == 9 ? 0 : 1;
    }

    // An alias names the earlier typedef's type, and works out nothing of
    // its own: its size is the earlier one's, and the length's call runs
    // once (`alias`).
    int a = 3;
    typedef char Aliased[a];
    a = 7;
    typedef Aliased Alias;
    typedef Aliased *Pointing;
    Alias alias;
    Aliased original;
    wrong += sizeof alias == 3 && sizeof original == 3 ? 0 : 1;
    calls = 0;
    typedef char Counted[counted()];
    typedef Counted Again;
    Again again;
    wrong += calls == 1 && sizeof again == 4 ? 0 : 1;

    // A qualified one too, its copy of the type keeping the length, in a
    // block whose own `q` hides the one the typedef named (`qualified`).
    int q = 3;
    typedef char Plain[q];
    {
        int q = 9;
        typedef const Plain Qualified;
        Qualified qualified;
        const Plain also;
        wrong += sizeof qualified == 3 && sizeof also == 3 && sizeof(const Plain) == 3 && q == 9
            ? 0
            : 1;
    }

    // Through pointers, nested arrays and a function's return, every length
    // the typedef's own declarator wrote is worked out (`returning`).
    stepped = 0;
    typedef int (*Rows)[step()];
    typedef int Grid[step()][step()];
    typedef int *Spread[step()];
    typedef char (*(*Returning)(void))[step()];
    wrong += stepped == 5 ? 0 : 1;
    return wrong;
}
