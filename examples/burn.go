// This program bursn as much CPU as it can, while measuring runtime lag.
//
// CPU burn is achieved by computing random collatz trajectories.
//
// Runtime lag is measured by consuming time.Ticker ticks at an expected 100ms interval,
// and then measuring the actual observed elapsed time between ticks.
// Doing so can measure various effects such as OS scheduling or pressure on the go runtime itself.
//
// We then report boxplot stats over a buffer of recent such measurements.
// The buffer is sized so that ic can contain samples from at least the last 10 seconds
// under normal rate (10hz from a 100ms ticker).

package main

import (
	"log"
	"math/rand"
	"runtime"
	"sort"
	"sync"
	"time"
)

func makeLagMonitor(every, keepLast time.Duration) *lagMonitor {
	bufferCap := keepLast / every
	return &lagMonitor{
		every:  every,
		buffer: make([]time.Time, bufferCap*2),
	}
}

type lagMonitor struct {
	every  time.Duration
	ticker *time.Ticker

	bufferLock sync.Mutex
	buffer     []time.Time
	cur        int
	full       bool
}

func (mon *lagMonitor) collect(t0, t1 time.Time) {
	mon.buffer[mon.cur] = t0
	mon.cur += 1

	mon.buffer[mon.cur] = t1
	mon.cur += 1

	if mon.cur >= len(mon.buffer) {
		mon.cur = mon.cur % len(mon.buffer)
		mon.full = true
	}
}

func (mon *lagMonitor) monitor() {
	ticker := time.NewTicker(mon.every)
	defer ticker.Stop()
	mon.ticker = ticker

	last := time.Now()
	for now := range ticker.C {
		mon.collect(last, now)
		last = now
	}
}

func (mon *lagMonitor) Start() {
	if mon.ticker == nil {
		go mon.monitor()
	}
}

func (mon *lagMonitor) Stop() {
	if mon.ticker != nil {
		mon.ticker.Stop()
	}
}

type lagData struct {
	Start    time.Time
	End      time.Time
	Actual   time.Duration
	Expected time.Duration
}

func (mon *lagMonitor) Data() []lagData {
	data := make([]lagData, 0, len(mon.buffer)/2)
	max := mon.cur
	if mon.full {
		max = len(mon.buffer)
	}

	for i := 0; i < max; i += 2 {
		j := i
		if mon.full {
			j = (mon.cur + i) % len(mon.buffer)
		}
		start := mon.buffer[j]
		end := mon.buffer[j+1]
		actual := end.Sub(start)
		data = append(data, lagData{start, end, actual, mon.every})
	}

	return data
}

func burnForever() {
	max := 0
	var last []int

	for true {

		var path []int
		n := rand.Intn(1_000_000_000)
		for n > 1 {
			path = append(path, n)
			if n%2 == 0 {
				n /= 2
			} else {
				n = 3*n + 1
			}
		}

		last = path
		if m := len(last); m > max {
			max = m
		}
	}
}

func main() {

	mon := makeLagMonitor(
		100*time.Millisecond,
		10*time.Second,
	)
	mon.Start()
	defer mon.Stop()

	numProcs := runtime.GOMAXPROCS(0)
	for i := 0; i < numProcs; i += 1 {
		go burnForever()
	}
	log.Printf("started %v burners", numProcs)

	threshold := 2 * time.Millisecond

	for range time.Tick(time.Second) {
		sample := mon.Data()
		sort.Slice(sample, func(i, j int) bool {
			return sample[i].Actual < sample[j].Actual
		})

		min := sample[0].Actual
		max := sample[len(sample)-1].Actual
		if max-min < threshold {
			log.Printf("[lag report] min:%v max:%v", min, max)
			continue
		}

		q25 := sample[len(sample)/4].Actual
		q50 := sample[len(sample)/2].Actual
		q75 := sample[len(sample)*3/4].Actual
		iqr := q75 - q25

		add := 3 * iqr
		if add < threshold {
			add = threshold
		}
		hi := q50 + add

		numOutliers := 0
		for _, s := range sample {
			if s.Actual >= hi {
				numOutliers += 1
			}
		}

		if numOutliers == 0 {
			log.Printf("[lag report] min:%v max:%v box:[ %v %v %v ] no outliers within threshold:%v", min, max, q25, q50, q75, threshold)
			continue
		}

		log.Printf(
			"[lag report] min:%v max:%v box:[ %v %v %v ] hi:%v hiOutliers:%v %.1f%%",
			min, max,
			q25, q50, q75, hi,
			numOutliers,
			float64(numOutliers)/float64(len(sample))*100,
		)
		for _, s := range sample {
			if s.Actual >= hi {
				log.Printf("%+v", s)
			}
		}
	}
}
